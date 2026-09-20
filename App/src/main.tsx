import { Component, type ErrorInfo, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles.css";

class AppErrorBoundary extends Component<
  { children: ReactNode },
  { error: string | null }
> {
  state: { error: string | null } = { error: null };

  static getDerivedStateFromError(error: unknown) {
    return { error: error instanceof Error ? error.message : String(error) };
  }

  componentDidCatch(error: Error, information: ErrorInfo) {
    console.error(
      "The application interface failed.",
      error,
      information.componentStack,
    );
  }

  render() {
    if (this.state.error !== null) {
      return (
        <main className="fatal-error" role="alert">
          <h1>Litematica Preview could not display its interface.</h1>
          <p>Your schematic files have not been changed.</p>
          <pre>{this.state.error}</pre>
          <p>
            Close and reopen the application to try again. If this keeps
            happening, reinstall the complete application.
          </p>
        </main>
      );
    }
    return this.props.children;
  }
}

const root = document.getElementById("root");
if (!root) throw new Error("The application root element is missing.");
createRoot(root).render(
  <AppErrorBoundary>
    <App />
  </AppErrorBoundary>,
);
