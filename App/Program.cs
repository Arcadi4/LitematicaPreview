namespace LitematicaPreview;

internal static class Program
{
    [STAThread]
    private static void Main(string[] args)
    {
        ApplicationConfiguration.Initialize();
        Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
        Application.ThreadException += (_, e) => MessageBox.Show(e.Exception.Message,
            "Litematica Preview", MessageBoxButtons.OK, MessageBoxIcon.Error);
        try
        {
            if (args is ["--register"])
            {
                FileAssociations.Register();
                return;
            }
            if (args is ["--unregister"])
            {
                FileAssociations.Unregister();
                return;
            }
            // Explorer passes the quoted filename as one argument, including
            // Unicode and spaces. Each launch owns one independent window.
            if (args.Length > 1)
                throw new ArgumentException("Open one schematic per window.");
            Application.Run(new MainForm(args.FirstOrDefault()));
        }
        catch (Exception error)
        {
            MessageBox.Show(error.Message, "Litematica Preview", MessageBoxButtons.OK, MessageBoxIcon.Error);
            Environment.ExitCode = 1;
        }
    }
}
