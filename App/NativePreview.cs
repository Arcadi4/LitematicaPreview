using System.IO.MemoryMappedFiles;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;
using OpenTK.Mathematics;

namespace LitematicaPreview;

internal sealed class NativePreview : SafeHandleZeroOrMinusOneIsInvalid
{
    internal static readonly string[] Extensions =
        [".litematic", ".schem", ".schematic", ".nbt", ".snbt", ".mcstructure", ".nusn"];

    // The pack is immutable and reused. Serial work bounds peak memory, and
    // cancellation drops queued opens before another decode/mesh begins.
    private static readonly SemaphoreSlim LoadGate = new(1, 1);
    private static readonly Lazy<PackHandle> Pack = new(OpenPack);
    private NativePreview(nint value) : base(true) => SetHandle(value);
    protected override bool ReleaseHandle() { PreviewFree(handle); return true; }

    internal static bool Supports(string path) =>
        Extensions.Contains(Path.GetExtension(path), StringComparer.OrdinalIgnoreCase);

    internal static async Task<NativePreview> LoadAsync(string path, CancellationToken cancellation)
    {
        if (!Supports(path))
            throw new InvalidDataException($"Supported files: {string.Join(", ", Extensions)}.");
        await LoadGate.WaitAsync(cancellation).ConfigureAwait(false);
        try { return await Task.Run(() => Load(path, cancellation), cancellation).ConfigureAwait(false); }
        finally { LoadGate.Release(); }
    }

    private static unsafe NativePreview Load(string path, CancellationToken cancellation)
    {
        cancellation.ThrowIfCancellationRequested();
        var pack = Pack.Value;
        cancellation.ThrowIfCancellationRequested();
        // The mapped file is stable for the entire native call and never copied
        // into a large managed byte[]. Rust owns only the decoded schematic.
        using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read);
        if (file.Length is <= 0 or > 1_073_741_824)
            throw new InvalidDataException("Choose a nonempty schematic no larger than 1 GiB.");
        using var mapping = MemoryMappedFile.CreateFromFile(file, null, 0, MemoryMappedFileAccess.Read, HandleInheritability.None, true);
        using var view = mapping.CreateViewAccessor(0, file.Length, MemoryMappedFileAccess.Read);
        byte* pointer = null;
        view.SafeMemoryMappedViewHandle.AcquirePointer(ref pointer);
        try
        {
            Check(PreviewLoad(pack, (nint)(pointer + view.PointerOffset), (nuint)file.Length, out var result, out var error), error);
            var preview = new NativePreview(result);
            if (cancellation.IsCancellationRequested)
            {
                preview.Dispose();
                cancellation.ThrowIfCancellationRequested();
            }
            return preview;
        }
        finally { view.SafeMemoryMappedViewHandle.ReleasePointer(); }
    }

    private static unsafe PackHandle OpenPack()
    {
        var bytes = File.ReadAllBytes(Path.Combine(AppContext.BaseDirectory, "Assets", "pack.zip"));
        fixed (byte* pointer = bytes)
        {
            Check(PackOpen((nint)pointer, (nuint)bytes.Length, out var pack, out var error), error);
            return new PackHandle(pack);
        }
    }

    private static unsafe void Check(int status, ErrorBuffer error)
    {
        try
        {
            if (status != 0)
            {
                var message = error.Data == 0 ? "Unable to load this schematic."
                    : Encoding.UTF8.GetString(new ReadOnlySpan<byte>((void*)error.Data, checked((int)error.Length)));
                throw new InvalidDataException(message);
            }
        }
        finally { if (error.Data != 0) ErrorFree(error.Data, error.Length); }
    }

    internal Info GetInfo()
    {
        if (PreviewInfo(this, out var value) != 0) throw new InvalidDataException("Invalid preview handle.");
        return value;
    }

    internal Part GetPart(uint index)
    {
        if (PreviewPart(this, index, out var value) != 0) throw new InvalidDataException("Missing mesh part.");
        return value;
    }

    internal Texture GetTexture(uint index)
    {
        if (PreviewTexture(this, index, out var value) != 0) throw new InvalidDataException("Missing block texture.");
        return value;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct Info
    {
        public long BlockCount, BlockEntityCount;
        public ulong TriangleCount;
        public uint PartCount, TextureCount;
        public Vector3 Min, Max;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct Part
    {
        public nint Positions, Normals, Uvs, Colors, Indices;
        public uint VertexCount, IndexCount, TextureIndex, AlphaMode;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct Texture
    {
        public nint Pixels;
        public nuint ByteCount;
        public uint Width, Height;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ErrorBuffer { public nint Data; public nuint Length; }

    private sealed class PackHandle : SafeHandleZeroOrMinusOneIsInvalid
    {
        internal PackHandle(nint value) : base(true) => SetHandle(value);
        protected override bool ReleaseHandle() { PackFree(handle); return true; }
    }

    private const string Library = "litematica_preview_native";
    [DllImport(Library, EntryPoint = "lp_pack_open", CallingConvention = CallingConvention.Cdecl)]
    private static extern int PackOpen(nint data, nuint length, out nint pack, out ErrorBuffer error);
    [DllImport(Library, EntryPoint = "lp_pack_free", CallingConvention = CallingConvention.Cdecl)]
    private static extern void PackFree(nint pack);
    [DllImport(Library, EntryPoint = "lp_preview_load", CallingConvention = CallingConvention.Cdecl)]
    private static extern int PreviewLoad(PackHandle pack, nint data, nuint length, out nint preview, out ErrorBuffer error);
    [DllImport(Library, EntryPoint = "lp_preview_free", CallingConvention = CallingConvention.Cdecl)]
    private static extern void PreviewFree(nint preview);
    [DllImport(Library, EntryPoint = "lp_preview_info", CallingConvention = CallingConvention.Cdecl)]
    private static extern int PreviewInfo(NativePreview preview, out Info info);
    [DllImport(Library, EntryPoint = "lp_preview_part", CallingConvention = CallingConvention.Cdecl)]
    private static extern int PreviewPart(NativePreview preview, uint index, out Part part);
    [DllImport(Library, EntryPoint = "lp_preview_texture", CallingConvention = CallingConvention.Cdecl)]
    private static extern int PreviewTexture(NativePreview preview, uint index, out Texture texture);
    [DllImport(Library, EntryPoint = "lp_error_free", CallingConvention = CallingConvention.Cdecl)]
    private static extern void ErrorFree(nint data, nuint length);
}
