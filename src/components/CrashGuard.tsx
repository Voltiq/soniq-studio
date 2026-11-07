import { Component, type ReactNode } from "react";
import { backend } from "../engine/backend";

/** Interface errors nobody caught: kept as reports (only if the user switched them on, a few per run). */
export function keepInterfaceErrors() {
  window.addEventListener("error", (e) => {
    void backend.crashUi(String(e.message || e.error || "Unknown error"), e.error instanceof Error ? e.error.stack : undefined).catch(() => {});
  });
  window.addEventListener("unhandledrejection", (e) => {
    // Commands that fail reject with a plain message the UI already shows; only real errors count.
    if (e.reason instanceof Error) void backend.crashUi(e.reason.message, e.reason.stack).catch(() => {});
  });
}

/** If drawing the interface fails, say so (instead of a blank window) and offer a reload: the song lives in Rust and survives it. */
export class CrashGuard extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: { componentStack?: string | null }) {
    void backend.crashUi(error.message, `${error.stack ?? ""}\n\nComponents:${info.componentStack ?? ""}`).catch(() => {});
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="crash-guard" role="alert">
        <div className="card">
          <h1>Something went wrong drawing Soniq</h1>
          <p className="muted">
            Your song is safe: it lives in the audio engine, and autosave keeps it. Reload the interface to carry on. If you keep crash reports, this one
            is in Help › Crash reports.
          </p>
          <code className="muted">{this.state.error.message}</code>
          <button className="btn btn-primary" type="button" onClick={() => window.location.reload()}>
            Reload the interface
          </button>
        </div>
      </div>
    );
  }
}
