using System.Diagnostics;

namespace LitematicaPreview;

internal sealed class MainForm : Form
{
    private readonly Panel content = new() { Dock = DockStyle.Fill };
    private readonly FlowLayoutPanel welcome = new()
    {
        Dock = DockStyle.Fill, FlowDirection = FlowDirection.TopDown, WrapContents = false,
        AutoScroll = true, Padding = new Padding(36), BackColor = SystemColors.Window,
    };
    private readonly ToolStripStatusLabel status = new("Open a schematic to begin.") { Spring = true, TextAlign = ContentAlignment.MiddleLeft };
    private readonly ToolStripButton cancel = new("Cancel") { Visible = false };
    private readonly ToolStripProgressBar progress = new() { Style = ProgressBarStyle.Marquee, Visible = false };
    private readonly ToolStripButton fit = new("Fit view") { Enabled = false, ToolTipText = "Fit the schematic (F)" };
    private readonly ToolStripMenuItem defaults = new("Set file associations…");
    private SchematicView? viewer;
    private CancellationTokenSource? loading;
    private int generation;
    private bool closing;

    internal MainForm(string? initialPath)
    {
        Text = "Litematica Preview";
        ClientSize = new Size(1000, 720);
        MinimumSize = new Size(700, 500);
        AutoScaleMode = AutoScaleMode.Dpi;
        AllowDrop = true;
        Icon = Icon.ExtractAssociatedIcon(Environment.ProcessPath!);
        var menu = new MenuStrip();
        var file = new ToolStripMenuItem("&File");
        var open = new ToolStripMenuItem("&Open…", null, (_, _) => ChooseFile()) { ShortcutKeys = Keys.Control | Keys.O };
        file.DropDownItems.Add(open);
        file.DropDownItems.Add("&Home", null, (_, _) => Home());
        file.DropDownItems.Add(new ToolStripSeparator());
        file.DropDownItems.Add(defaults);
        file.DropDownItems.Add("Remove file associations", null, (_, _) => Unregister());
        file.DropDownItems.Add(new ToolStripSeparator());
        file.DropDownItems.Add("E&xit", null, (_, _) => Close());
        var help = new ToolStripMenuItem("&Help");
        help.DropDownItems.Add("Controls", null, (_, _) => MessageBox.Show(this,
            "Drag to orbit · Right-drag to pan · Scroll to zoom\n\nArrow keys orbit · Shift + arrows pan\n+ / − zoom · F fits the model · Ctrl+O opens a file",
            "Controls", MessageBoxButtons.OK, MessageBoxIcon.Information));
        help.DropDownItems.Add("About and licenses", null, (_, _) => About());
        menu.Items.Add(file); menu.Items.Add(help);
        MainMenuStrip = menu;
        var toolbar = new ToolStrip { GripStyle = ToolStripGripStyle.Hidden };
        toolbar.Items.Add(new ToolStripButton("Open…", null, (_, _) => ChooseFile()));
        toolbar.Items.Add(new ToolStripButton("Home", null, (_, _) => Home()));
        toolbar.Items.Add(fit);
        var footer = new StatusStrip();
        footer.Items.Add(status); footer.Items.Add(progress); footer.Items.Add(cancel);
        fit.Click += (_, _) => viewer?.Fit();
        cancel.Click += (_, _) => Home();
        defaults.Click += (_, _) => Register();
        Controls.Add(content); Controls.Add(toolbar); Controls.Add(menu); Controls.Add(footer);
        content.Controls.Add(welcome);
        BuildWelcome();
        EnableFileDrop(this);
        EnableFileDrop(content);
        EnableFileDrop(welcome);
        Shown += async (_, _) => { if (initialPath is not null) await OpenAsync(initialPath); };
        FormClosing += (_, _) => { closing = true; generation++; loading?.Cancel(); };
    }

    private void BuildWelcome()
    {
        var imagePath = Path.Combine(AppContext.BaseDirectory, "Assets", "icon.png");
        if (File.Exists(imagePath))
        {
            var icon = new PictureBox { Width = 100, Height = 100, SizeMode = PictureBoxSizeMode.Zoom, Image = Image.FromFile(imagePath) };
            icon.Disposed += (_, _) => icon.Image?.Dispose();
            welcome.Controls.Add(icon);
        }
        welcome.Controls.Add(new Label { Text = "Litematica Preview", AutoSize = true, Font = new Font(Font.FontFamily, 25, FontStyle.Bold), Margin = new Padding(0, 16, 0, 8) });
        welcome.Controls.Add(new Label { Text = "Minecraft schematics and structures, on Windows.", AutoSize = true, Margin = new Padding(0, 0, 0, 20) });
        var open = new Button { Text = "Open schematic…", AutoSize = true, Padding = new Padding(14, 8, 14, 8) };
        open.Click += (_, _) => ChooseFile();
        welcome.Controls.Add(open);
        welcome.Controls.Add(new Label { Text = "Bundled demos", AutoSize = true, Margin = new Padding(0, 24, 0, 8) });
        var demos = new FlowLayoutPanel { Width = 560, Height = 110, WrapContents = true, AutoSize = true, MaximumSize = new Size(560, 0) };
        var directory = Path.Combine(AppContext.BaseDirectory, "Demos");
        if (Directory.Exists(directory))
        {
            foreach (string extension in NativePreview.Extensions)
            foreach (string path in Directory.EnumerateFiles(directory).Where(p => Path.GetExtension(p).Equals(extension, StringComparison.OrdinalIgnoreCase)))
            {
                var button = new Button { Text = $"{Path.GetFileNameWithoutExtension(path)}\n{extension}", Width = 128, Height = 52 };
                button.Click += async (_, _) => await OpenAsync(path);
                demos.Controls.Add(button);
            }
        }
        welcome.Controls.Add(demos);
        welcome.Controls.Add(new Label
        {
            AutoSize = true, MaximumSize = new Size(560, 0), Margin = new Padding(0, 20, 0, 10),
            Text = "Drop a file here, or use File → Set file associations to select Litematica Preview as its default app. Then double-click it in Explorer.\n\nDrag to orbit · Scroll to zoom · Right-drag to pan\n\nYour schematics stay on this PC. Rendering works offline.",
        });
    }

    private async void ChooseFile()
    {
        using var dialog = new OpenFileDialog
        {
            Title = "Open Minecraft schematic", CheckFileExists = true,
            Filter = $"Minecraft schematics|{string.Join(';', NativePreview.Extensions.Select(e => "*" + e))}|All files|*.*",
        };
        if (dialog.ShowDialog(this) == DialogResult.OK) await OpenAsync(dialog.FileName);
    }

    private async Task OpenAsync(string path)
    {
        loading?.Cancel();
        using var cancellation = new CancellationTokenSource();
        loading = cancellation;
        int current = ++generation;
        viewer?.ClearModel();
        fit.Enabled = false;
        progress.Visible = cancel.Visible = true;
        status.ForeColor = SystemColors.ControlText;
        status.Text = $"Loading {Path.GetFileName(path)}…";
        var elapsed = Stopwatch.StartNew();
        try
        {
            using var preview = await NativePreview.LoadAsync(path, cancellation.Token);
            cancellation.Token.ThrowIfCancellationRequested();
            if (closing || current != generation) return;
            if (viewer is null)
            {
                viewer = new SchematicView();
                viewer.RenderFailed += ShowError;
                EnableFileDrop(viewer);
                content.Controls.Add(viewer);
            }
            welcome.Visible = false;
            viewer.Visible = true;
            viewer.BringToFront();
            status.Text = "Uploading the schematic to the graphics device…";
            await viewer.UploadAsync(preview, cancellation.Token);
            if (closing || current != generation) return;
            var info = preview.GetInfo();
            var size = info.Max - info.Min;
            Text = $"{Path.GetFileName(path)} — Litematica Preview";
            status.Text = $"{info.BlockCount:N0} blocks · {size.X:0.##} × {size.Y:0.##} × {size.Z:0.##} · {info.TriangleCount:N0} triangles · Loaded in {elapsed.Elapsed.TotalSeconds:0.00}s";
            fit.Enabled = true;
            viewer.Focus();
        }
        catch (OperationCanceledException) { }
        catch (Exception error)
        {
            if (closing || current != generation) return;
            viewer?.ClearModel();
            welcome.Visible = true; welcome.BringToFront();
            ShowError(error is DllNotFoundException or BadImageFormatException
                ? "The Windows native decoder is missing or has the wrong architecture. Reinstall the complete x64 build."
                : error.Message);
        }
        finally
        {
            if (!closing && current == generation)
            {
                progress.Visible = cancel.Visible = false;
                loading = null;
            }
        }
    }

    private void Home()
    {
        generation++; loading?.Cancel(); loading = null;
        viewer?.ClearModel();
        if (viewer is not null) viewer.Visible = false;
        welcome.Visible = true; welcome.BringToFront();
        progress.Visible = cancel.Visible = fit.Enabled = false;
        status.ForeColor = SystemColors.ControlText;
        status.Text = "Open a schematic to begin.";
        Text = "Litematica Preview";
    }

    private void ShowError(string message)
    {
        status.ForeColor = Color.Firebrick;
        status.Text = message;
        status.ToolTipText = message;
    }

    private void EnableFileDrop(Control control)
    {
        control.AllowDrop = true;
        control.DragEnter += (_, e) => e.Effect = e.Data?.GetData(DataFormats.FileDrop) is string[] paths
            && paths.Any(NativePreview.Supports) ? DragDropEffects.Copy : DragDropEffects.None;
        control.DragDrop += async (_, e) =>
        {
            if (e.Data?.GetData(DataFormats.FileDrop) is string[] paths
                && paths.FirstOrDefault(NativePreview.Supports) is { } path) await OpenAsync(path);
        };
    }

    private void Register()
    {
        try
        {
            FileAssociations.Register();
            FileAssociations.OpenSettings();
            status.Text = "Choose Litematica Preview for your schematic extensions in Windows Settings.";
        }
        catch (Exception error) { ShowError(error.Message); }
    }

    private void Unregister()
    {
        try { FileAssociations.Unregister(); status.Text = "This copy's file associations were removed."; }
        catch (Exception error) { ShowError(error.Message); }
    }

    private void About()
    {
        MessageBox.Show(this,
            "Litematica Preview 0.1.0\nA Windows C# port of LitematicaQL.\n\nPowered by Nucleation and OpenTK.\nDistributed under GNU AGPL v3; no warranty.\nYou may redistribute under the terms in LICENSE.\n\nLICENSE, NOTICE, and ThirdParty notices are included beside the application.",
            "About Litematica Preview", MessageBoxButtons.OK, MessageBoxIcon.Information);
    }
}
