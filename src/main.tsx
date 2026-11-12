import { Component, type ErrorInfo, type ReactNode } from "react"
import { createRoot } from "react-dom/client"

import App from "./App"

import "./styles.css"

class AppErrorBoundary extends Component<
  { children: ReactNode },
  { error: string | null; recovered: string | null }
> {
  state: { error: string | null; recovered: string | null } = { error: null, recovered: null }

  static getDerivedStateFromError(error: unknown) {
    return { error: error instanceof Error ? error.message : String(error) }
  }

  componentDidCatch(error: Error, information: ErrorInfo) {
    console.error("The application interface failed.", error, information.componentStack)
    if (this.state.recovered === null) {
      this.setState({ error: null, recovered: error.message })
    }
  }

  render() {
    if (this.state.error !== null) {
      return (
        <main
          className="mx-auto max-h-full max-w-2xl overflow-auto bg-[Canvas] p-6 leading-relaxed text-[CanvasText]"
          role="alert"
        >
          <h1 className="mb-4 text-2xl leading-snug font-bold">
            Litematica Preview could not display its interface.
          </h1>
          <p className="mb-4">Your schematic files have not been changed.</p>
          <pre className="my-4 rounded-md border border-[GrayText] p-4 break-words whitespace-pre-wrap">
            {this.state.error}
          </pre>
          <button onClick={() => this.setState({ recovered: this.state.error, error: null })}>
            Return home
          </button>
        </main>
      )
    }
    if (this.state.recovered !== null) return <App initialError={this.state.recovered} />
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
