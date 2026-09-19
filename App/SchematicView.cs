using OpenTK.GLControl;
using OpenTK.Graphics.OpenGL4;
using OpenTK.Mathematics;
using OpenTK.Windowing.Common;

namespace LitematicaPreview;

internal sealed class SchematicView : GLControl
{
    private sealed class MeshPart
    {
        internal int Vao, Count, Texture;
        internal uint AlphaMode;
        internal readonly int[] Buffers = new int[5];
    }

    private readonly List<MeshPart> parts = [];
    private readonly List<int> textures = [];
    private int program, matrixLocation, offsetLocation, alphaLocation, gridLocation;
    private int gridVao, gridBuffer, gridCount;
    private Vector3 centre, minimum, maximum, target;
    private float yaw = MathHelper.PiOver4, pitch = MathF.Atan(1 / MathF.Sqrt(2));
    private float distance = 10, fittedDistance = 10;
    private Point lastMouse;
    private bool hasModel, failed;
    internal event Action<string>? RenderFailed;

    internal SchematicView() : base(new GLControlSettings
    {
        API = ContextAPI.OpenGL, APIVersion = new Version(3, 3), Profile = ContextProfile.Core,
        NumberOfSamples = 4, SrgbCapable = true, IsEventDriven = true,
    })
    {
        Dock = DockStyle.Fill;
        TabStop = true;
        AccessibleName = "Schematic view";
        AccessibleDescription = "Drag to orbit, right-drag to pan, scroll to zoom. Arrow keys orbit, Shift and arrows pan, plus and minus zoom, F fits the model.";
    }

    private void InitializeRenderer()
    {
        MakeCurrent();
        if (program != 0) return;
        var vertex = Compile(ShaderType.VertexShader, VertexSource);
        int fragment = 0, linked = 0;
        try
        {
            fragment = Compile(ShaderType.FragmentShader, FragmentSource);
            linked = GL.CreateProgram();
            GL.AttachShader(linked, vertex);
            GL.AttachShader(linked, fragment);
            GL.LinkProgram(linked);
            GL.GetProgram(linked, GetProgramParameterName.LinkStatus, out int success);
            if (success == 0) throw new InvalidOperationException(GL.GetProgramInfoLog(linked));
            program = linked;
        }
        catch { if (linked != 0) GL.DeleteProgram(linked); throw; }
        finally { GL.DeleteShader(vertex); if (fragment != 0) GL.DeleteShader(fragment); }
        matrixLocation = GL.GetUniformLocation(program, "mvp");
        offsetLocation = GL.GetUniformLocation(program, "offset");
        alphaLocation = GL.GetUniformLocation(program, "alphaMode");
        gridLocation = GL.GetUniformLocation(program, "isGrid");
        GL.UseProgram(program);
        GL.Uniform1(GL.GetUniformLocation(program, "blockTexture"), 0);
        GL.Enable(EnableCap.DepthTest);
        GL.Enable(EnableCap.Multisample);
        GL.Enable(EnableCap.FramebufferSrgb);
        GL.FrontFace(FrontFaceDirection.Ccw);
        GL.CullFace(TriangleFace.Back);
    }

    internal async Task UploadAsync(NativePreview preview, CancellationToken cancellation)
    {
        cancellation.ThrowIfCancellationRequested();
        InitializeRenderer();
        var info = preview.GetInfo();
        var newParts = new List<MeshPart>();
        var newTextures = new List<int>();
        try
        {
            GL.GetInteger(GetPName.MaxTextureSize, out int maximumTextureSize);
            for (uint i = 0; i < info.TextureCount; i++)
            {
                var texture = preview.GetTexture(i);
                if (texture.Width == 0 || texture.Height == 0
                    || texture.Width > maximumTextureSize || texture.Height > maximumTextureSize
                    || (ulong)texture.ByteCount != (ulong)texture.Width * texture.Height * 4)
                    throw new InvalidDataException("A block texture exceeds the graphics device's limits.");
                int id = GL.GenTexture();
                newTextures.Add(id);
                GL.BindTexture(TextureTarget.Texture2D, id);
                GL.TexImage2D(TextureTarget.Texture2D, 0, PixelInternalFormat.Srgb8Alpha8,
                    (int)texture.Width, (int)texture.Height, 0, PixelFormat.Rgba, PixelType.UnsignedByte, texture.Pixels);
                // Pixel-art texels stay sharp. Atlas clamps, greedy materials tile.
                GL.TexParameter(TextureTarget.Texture2D, TextureParameterName.TextureMinFilter, (int)TextureMinFilter.Nearest);
                GL.TexParameter(TextureTarget.Texture2D, TextureParameterName.TextureMagFilter, (int)TextureMagFilter.Nearest);
                int wrap = (int)(i == 0 ? TextureWrapMode.ClampToEdge : TextureWrapMode.Repeat);
                GL.TexParameter(TextureTarget.Texture2D, TextureParameterName.TextureWrapS, wrap);
                GL.TexParameter(TextureTarget.Texture2D, TextureParameterName.TextureWrapT, wrap);
                CheckGraphics();
            }
            for (uint i = 0; i < info.PartCount; i++)
            {
                cancellation.ThrowIfCancellationRequested();
                MakeCurrent();
                var source = preview.GetPart(i);
                var part = new MeshPart
                {
                    Vao = GL.GenVertexArray(), Count = checked((int)source.IndexCount),
                    Texture = newTextures[checked((int)source.TextureIndex)], AlphaMode = source.AlphaMode,
                };
                newParts.Add(part);
                GL.BindVertexArray(part.Vao);
                GL.GenBuffers(5, part.Buffers);
                Attribute(0, 3, source.Positions);
                Attribute(1, 3, source.Normals);
                Attribute(2, 2, source.Uvs);
                Attribute(3, 4, source.Colors);
                GL.BindBuffer(BufferTarget.ElementArrayBuffer, part.Buffers[4]);
                GL.BufferData(BufferTarget.ElementArrayBuffer, checked((nint)((long)source.IndexCount * 4)), source.Indices, BufferUsageHint.StaticDraw);
                CheckGraphics();
                // Native data is borrowed only for synchronous GL uploads. No
                // large managed arrays, GLB, IPC, or intermediate serialization.
                await Task.Yield();

                void Attribute(int attribute, int size, nint pointer)
                {
                    GL.BindBuffer(BufferTarget.ArrayBuffer, part.Buffers[attribute]);
                    GL.BufferData(BufferTarget.ArrayBuffer, checked((nint)((long)source.VertexCount * size * 4)), pointer, BufferUsageHint.StaticDraw);
                    GL.EnableVertexAttribArray(attribute);
                    GL.VertexAttribPointer(attribute, size, VertexAttribPointerType.Float, false, 0, 0);
                }
            }
            cancellation.ThrowIfCancellationRequested();
            MakeCurrent();
            ClearModel();
            parts.AddRange(newParts);
            textures.AddRange(newTextures);
            newParts.Clear(); newTextures.Clear();
            centre = (info.Min + info.Max) / 2;
            minimum = info.Min - centre;
            maximum = info.Max - centre;
            hasModel = true;
            failed = false;
            BuildGrid();
            Fit();
        }
        finally
        {
            if (!IsDisposed && IsHandleCreated)
            {
                MakeCurrent();
                Release(newParts, newTextures);
                GL.BindVertexArray(0);
            }
            GC.KeepAlive(preview);
        }
    }

    internal void ClearModel()
    {
        if (program != 0 && IsHandleCreated && !IsDisposed)
        {
            MakeCurrent();
            Release(parts, textures);
            if (gridVao != 0) GL.DeleteVertexArray(gridVao);
            if (gridBuffer != 0) GL.DeleteBuffer(gridBuffer);
        }
        parts.Clear(); textures.Clear();
        gridVao = gridBuffer = gridCount = 0;
        hasModel = false;
        Invalidate();
    }

    internal void Fit()
    {
        if (!hasModel) return;
        yaw = MathHelper.PiOver4;
        pitch = MathF.Atan(1 / MathF.Sqrt(2));
        target = Vector3.Zero;
        fittedDistance = distance = FittingDistance();
        Invalidate();
    }

    private Vector3 Direction => new(MathF.Cos(pitch) * MathF.Sin(yaw), MathF.Sin(pitch), MathF.Cos(pitch) * MathF.Cos(yaw));

    private float FittingDistance()
    {
        var forward = -Direction;
        var right = Vector3.Normalize(Vector3.Cross(forward, Vector3.UnitY));
        var up = Vector3.Cross(right, forward);
        float vertical = MathF.Tan(MathHelper.DegreesToRadians(14)) / 1.18f;
        float horizontal = vertical * Math.Max(ClientSize.Width, 1) / Math.Max(ClientSize.Height, 1);
        float fit = 0.1f;
        foreach (float x in new[] { minimum.X, maximum.X })
        foreach (float y in new[] { minimum.Y, maximum.Y })
        foreach (float z in new[] { minimum.Z, maximum.Z })
        {
            var corner = new Vector3(x, y, z);
            float depth = Vector3.Dot(corner, forward);
            fit = Math.Max(fit, Math.Max(Math.Abs(Vector3.Dot(corner, up)) / vertical - depth,
                Math.Abs(Vector3.Dot(corner, right)) / horizontal - depth));
        }
        return fit;
    }

    protected override void OnResize(EventArgs e)
    {
        base.OnResize(e);
        if (hasModel)
        {
            float ratio = distance / fittedDistance;
            fittedDistance = FittingDistance();
            distance = fittedDistance * ratio;
        }
        Invalidate();
    }

    protected override void OnPaint(PaintEventArgs e)
    {
        base.OnPaint(e);
        if (failed || program == 0 || ClientSize.Width <= 0 || ClientSize.Height <= 0) return;
        try
        {
            MakeCurrent();
            GL.Viewport(0, 0, ClientSize.Width, ClientSize.Height);
            // Linear values yield the source app's #0b1016 in an sRGB buffer.
            GL.ClearColor(0.00335f, 0.00518f, 0.00802f, 1);
            GL.DepthMask(true);
            GL.Clear(ClearBufferMask.ColorBufferBit | ClearBufferMask.DepthBufferBit);
            if (hasModel)
            {
                var view = Matrix4.LookAt(target + Direction * distance, target, Vector3.UnitY);
                var projection = Matrix4.CreatePerspectiveFieldOfView(MathHelper.DegreesToRadians(28),
                    ClientSize.Width / (float)ClientSize.Height, Math.Max(fittedDistance / 1000, 0.01f), fittedDistance * 24);
                var mvp = view * projection;
                GL.UseProgram(program);
                GL.UniformMatrix4(matrixLocation, true, ref mvp);
                GL.ActiveTexture(TextureUnit.Texture0);
                GL.Disable(EnableCap.Blend);
                GL.Disable(EnableCap.CullFace);
                GL.Uniform1(gridLocation, 1);
                GL.Uniform3(offsetLocation, Vector3.Zero);
                GL.BindVertexArray(gridVao);
                GL.DrawArrays(PrimitiveType.Lines, 0, gridCount);
                GL.Uniform3(offsetLocation, -centre);
                GL.Uniform1(gridLocation, 0);
                foreach (var part in parts.Where(p => p.AlphaMode != 2)) Draw(part);
                GL.Enable(EnableCap.Blend);
                GL.BlendFunc(BlendingFactor.SrcAlpha, BlendingFactor.OneMinusSrcAlpha);
                GL.DepthMask(false);
                foreach (var part in parts.Where(p => p.AlphaMode == 2)) Draw(part);
                GL.DepthMask(true);
                GL.Disable(EnableCap.Blend);
                GL.BindVertexArray(0);
            }
            SwapBuffers();
            CheckGraphics();
        }
        catch (Exception error)
        {
            failed = true;
            RenderFailed?.Invoke($"The graphics device could not draw this schematic: {error.Message}");
        }
    }

    private void Draw(MeshPart part)
    {
        if (part.AlphaMode == 0) GL.Enable(EnableCap.CullFace);
        else GL.Disable(EnableCap.CullFace);
        GL.Uniform1(alphaLocation, (int)part.AlphaMode);
        GL.BindTexture(TextureTarget.Texture2D, part.Texture);
        GL.BindVertexArray(part.Vao);
        GL.DrawElements(PrimitiveType.Triangles, part.Count, DrawElementsType.UnsignedInt, 0);
    }

    private void BuildGrid()
    {
        float extent = Math.Max(Math.Max(maximum.X - minimum.X, maximum.Z - minimum.Z) * 1.15f, 16);
        float step = Math.Max(1, MathF.Ceiling(extent / 128));
        float edge = MathF.Ceiling(extent / (2 * step)) * step;
        float y = minimum.Y - 0.02f;
        var lines = new List<float>();
        for (float offset = -edge; offset <= edge; offset += step)
            lines.AddRange([offset, y, -edge, offset, y, edge, -edge, y, offset, edge, y, offset]);
        gridCount = lines.Count / 3;
        gridVao = GL.GenVertexArray(); gridBuffer = GL.GenBuffer();
        GL.BindVertexArray(gridVao);
        GL.BindBuffer(BufferTarget.ArrayBuffer, gridBuffer);
        GL.BufferData(BufferTarget.ArrayBuffer, lines.Count * sizeof(float), lines.ToArray(), BufferUsageHint.StaticDraw);
        GL.EnableVertexAttribArray(0);
        GL.VertexAttribPointer(0, 3, VertexAttribPointerType.Float, false, 0, 0);
        CheckGraphics();
    }

    protected override void OnMouseDown(MouseEventArgs e)
    {
        base.OnMouseDown(e);
        Focus(); lastMouse = e.Location; Capture = true;
    }
    protected override void OnMouseUp(MouseEventArgs e) { base.OnMouseUp(e); Capture = false; }
    protected override void OnMouseMove(MouseEventArgs e)
    {
        base.OnMouseMove(e);
        if (!hasModel || !Capture) return;
        int dx = e.X - lastMouse.X, dy = e.Y - lastMouse.Y;
        lastMouse = e.Location;
        if (e.Button == MouseButtons.Left)
        {
            yaw -= dx * 0.008f;
            pitch = Math.Clamp(pitch + dy * 0.008f, -1.5f, 1.5f);
        }
        else if (e.Button is MouseButtons.Right or MouseButtons.Middle) Pan(dx, dy);
        Invalidate();
    }
    private void Pan(float dx, float dy)
    {
        var right = Vector3.Normalize(Vector3.Cross(-Direction, Vector3.UnitY));
        var up = Vector3.Cross(right, -Direction);
        float scale = 2 * distance * MathF.Tan(MathHelper.DegreesToRadians(14)) / Math.Max(ClientSize.Height, 1);
        target += (-right * dx + up * dy) * scale;
    }
    protected override void OnMouseWheel(MouseEventArgs e)
    {
        base.OnMouseWheel(e);
        Zoom(MathF.Exp(-e.Delta / 120f * 0.12f));
    }
    private void Zoom(float factor)
    {
        distance = Math.Clamp(distance * factor, fittedDistance * 0.05f, fittedDistance * 8);
        Invalidate();
    }
    protected override bool IsInputKey(Keys keyData) =>
        (keyData & Keys.KeyCode) is Keys.Left or Keys.Right or Keys.Up or Keys.Down || base.IsInputKey(keyData);
    protected override void OnKeyDown(KeyEventArgs e)
    {
        base.OnKeyDown(e);
        if (!hasModel) return;
        var (dx, dy) = e.KeyCode switch
        {
            Keys.Left => (-18f, 0f), Keys.Right => (18f, 0f), Keys.Up => (0f, -18f), Keys.Down => (0f, 18f), _ => (0f, 0f),
        };
        if (dx != 0 || dy != 0)
        {
            if (e.Shift) Pan(dx, dy);
            else { yaw -= dx * 0.008f; pitch = Math.Clamp(pitch + dy * 0.008f, -1.5f, 1.5f); }
        }
        else if (e.KeyCode is Keys.Add or Keys.Oemplus) Zoom(0.88f);
        else if (e.KeyCode is Keys.Subtract or Keys.OemMinus) Zoom(1.12f);
        else if (e.KeyCode is Keys.F or Keys.Home) Fit();
        else return;
        e.Handled = e.SuppressKeyPress = true;
        Invalidate();
    }

    protected override void OnHandleDestroyed(EventArgs e)
    {
        // Free resources while the context still exists (base destroys it).
        if (program != 0)
        {
            ClearModel();
            MakeCurrent(); GL.DeleteProgram(program); program = 0;
        }
        base.OnHandleDestroyed(e);
    }

    private static void Release(IEnumerable<MeshPart> models, IEnumerable<int> maps)
    {
        foreach (var part in models) { GL.DeleteVertexArray(part.Vao); GL.DeleteBuffers(5, part.Buffers); }
        foreach (int id in maps) GL.DeleteTexture(id);
    }
    private static void CheckGraphics()
    {
        var error = GL.GetError();
        if (error != ErrorCode.NoError) throw new InvalidOperationException($"OpenGL: {error}. The model may exceed available graphics memory.");
    }
    private static int Compile(ShaderType type, string source)
    {
        int shader = GL.CreateShader(type);
        GL.ShaderSource(shader, source); GL.CompileShader(shader);
        GL.GetShader(shader, ShaderParameter.CompileStatus, out int success);
        if (success != 0) return shader;
        string error = GL.GetShaderInfoLog(shader);
        GL.DeleteShader(shader);
        throw new InvalidOperationException($"OpenGL 3.3 shader could not compile: {error}");
    }

    private const string VertexSource = """
        #version 330 core
        layout(location=0) in vec3 position;
        layout(location=1) in vec3 normal;
        layout(location=2) in vec2 uv;
        layout(location=3) in vec4 color;
        uniform mat4 mvp;
        uniform vec3 offset;
        out vec3 surfaceNormal;
        out vec2 textureUv;
        out vec4 tint;
        void main() {
            gl_Position = vec4(position + offset, 1.0) * mvp;
            surfaceNormal = normal; textureUv = uv; tint = color;
        }
        """;
    private const string FragmentSource = """
        #version 330 core
        in vec3 surfaceNormal;
        in vec2 textureUv;
        in vec4 tint;
        uniform sampler2D blockTexture;
        uniform int alphaMode;
        uniform bool isGrid;
        out vec4 pixel;
        void main() {
            if (isGrid) { pixel = vec4(0.020, 0.030, 0.042, 1.0); return; }
            vec4 base = texture(blockTexture, textureUv) * tint;
            if (alphaMode == 1 && base.a < 0.5) discard;
            vec3 n = normalize(surfaceNormal) * (gl_FrontFacing ? 1.0 : -1.0);
            float light = 0.68 + 0.32 * max(dot(n, normalize(vec3(1.0, 1.4, 0.8))), 0.0);
            pixel = vec4(base.rgb * light, alphaMode == 2 ? base.a : 1.0);
        }
        """;
}
