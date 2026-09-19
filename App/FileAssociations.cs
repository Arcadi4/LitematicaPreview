using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.Win32;

namespace LitematicaPreview;

internal static class FileAssociations
{
    private const string ProgId = "LitematicaPreview.Schematic";
    private const string AppKey = @"Software\LitematicaPreview";
    private const string Classes = @"Software\Classes\";

    internal static void Register()
    {
        var executable = Environment.ProcessPath ?? throw new InvalidOperationException("Application path unavailable.");
        var command = $"\"{executable}\" \"%1\"";
        Set(Classes + ProgId, "", "Minecraft schematic");
        Set(Classes + ProgId + @"\DefaultIcon", "", $"\"{executable}\",0");
        Set(Classes + ProgId + @"\shell\open\command", "", command);
        Set(Classes + @"Applications\LitematicaPreview.exe", "FriendlyAppName", "Litematica Preview");
        Set(Classes + @"Applications\LitematicaPreview.exe\shell\open\command", "", command);
        Set(AppKey + @"\Capabilities", "ApplicationName", "Litematica Preview");
        Set(AppKey + @"\Capabilities", "ApplicationDescription", "View Minecraft schematics and structures offline.");
        foreach (var extension in NativePreview.Extensions)
        {
            // Provide the initial handler for an unclaimed extension. Existing
            // defaults, including Windows' protected UserChoice, stay intact.
            using var existing = Registry.ClassesRoot.OpenSubKey(extension);
            if (string.IsNullOrEmpty(existing?.GetValue("") as string))
                Set(Classes + extension, "", ProgId);
            Set(Classes + extension + @"\OpenWithProgids", ProgId, "", RegistryValueKind.None);
            Set(Classes + @"Applications\LitematicaPreview.exe\SupportedTypes", extension, "");
            Set(AppKey + @"\Capabilities\FileAssociations", extension, ProgId);
        }
        Set(@"Software\RegisteredApplications", "Litematica Preview", AppKey + @"\Capabilities");
        NotifyShell();
    }

    internal static void OpenSettings() => Process.Start(new ProcessStartInfo(
        OperatingSystem.IsWindowsVersionAtLeast(10, 0, 22621)
            ? "ms-settings:defaultapps?registeredAppUser=Litematica%20Preview"
            : "ms-settings:defaultapps") { UseShellExecute = true });

    internal static void Unregister()
    {
        // A portable copy must not unregister a different installed copy.
        using var command = Registry.CurrentUser.OpenSubKey(Classes + ProgId + @"\shell\open\command");
        var expected = $"\"{Environment.ProcessPath}\" \"%1\"";
        if (!string.Equals(command?.GetValue("") as string, expected, StringComparison.OrdinalIgnoreCase)) return;
        foreach (var extension in NativePreview.Extensions)
        {
            using var type = Registry.CurrentUser.OpenSubKey(Classes + extension, true);
            if (string.Equals(type?.GetValue("") as string, ProgId, StringComparison.Ordinal))
                type!.DeleteValue("", false);
            using var key = Registry.CurrentUser.OpenSubKey(Classes + extension + @"\OpenWithProgids", true);
            key?.DeleteValue(ProgId, false);
        }
        Registry.CurrentUser.DeleteSubKeyTree(Classes + ProgId, false);
        Registry.CurrentUser.DeleteSubKeyTree(Classes + @"Applications\LitematicaPreview.exe", false);
        Registry.CurrentUser.DeleteSubKeyTree(AppKey, false);
        using var registered = Registry.CurrentUser.OpenSubKey(@"Software\RegisteredApplications", true);
        registered?.DeleteValue("Litematica Preview", false);
        NotifyShell();
    }

    private static void Set(string path, string name, string value, RegistryValueKind kind = RegistryValueKind.String)
    {
        using var key = Registry.CurrentUser.CreateSubKey(path);
        key.SetValue(name, kind == RegistryValueKind.None ? Array.Empty<byte>() : value, kind);
    }

    private static void NotifyShell() => SHChangeNotify(0x08000000, 0, 0, 0);
    [DllImport("shell32.dll")]
    private static extern void SHChangeNotify(uint eventId, uint flags, nint item1, nint item2);
}
