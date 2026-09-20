import { Component, type ErrorInfo, type ReactNode } from "react"
import { createRoot } from "react-dom/client"
import App from "./App"
import "./styles.css"

class AppErrorBoundary extends Component<{ children: ReactNode }, { error: string | null }> {
  state: { error: string | null } = { error: null }

  static getDerivedStateFromError(error: unknown) {
    return { error: error instanceof Error ? error.message : String(error) }
  }

  componentDidCatch(error: Error, information: ErrorInfo) {
    console.error("The application interface failed.", error, information.componentStack)
  }

  render() {
    if (this.state.error !== null) {
      return (
        <main
          className="mx-auto max-h-full max-w-2xl overflow-auto p-6 text-[CanvasText] bg-[Canvas] leading-relaxed"
          role="alert"
        >
          <h1 className="text-2xl font-bold leading-snug mb-4">
            Litematica Preview could not display its interface.
          </h1>
          <p className="mb-4">Your schematic files have not been changed.</p>
          <pre className="my-4 p-4 border border-[GrayText] rounded-md whitespace-pre-wrap break-words">
            {this.state.error}
          </pre>
          <p>
            Close and reopen the application to try again. If this keeps happening, reinstall the
            complete application.
          </p>
        </main>
      )
    }
    return this.props.children
  }
}

const root = document.getElementById("root")
if (!root) throw new Error("The application root element is missing.")
createRoot(root).render(
  <AppErrorBoundary>
    <App />
  </AppErrorBoundary>,
)
