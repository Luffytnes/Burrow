import { Component, type ErrorInfo, type ReactNode } from "react";

export default class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  constructor(props: { children: ReactNode }) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("App crash:", error, info);
  }

  render() {
    if (!this.state.error) return this.props.children;

    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          justifyContent: "center",
          height: "100vh",
          padding: "2rem",
          fontFamily: "monospace",
          background: "#131110",
          color: "#f2ede8",
        }}
      >
        <div style={{ fontSize: 14, color: "#f88779", marginBottom: 12, fontWeight: 700 }}>
          Erreur de rendu
        </div>
        <pre
          style={{
            fontSize: 11,
            color: "#aaa",
            maxWidth: 600,
            whiteSpace: "pre-wrap",
            textAlign: "left",
          }}
        >
          {this.state.error.message}
        </pre>
        <button
          onClick={() => this.setState({ error: null })}
          style={{
            marginTop: 20,
            padding: "6px 16px",
            background: "#e54857",
            color: "#ffffff",
            border: "none",
            borderRadius: 6,
            cursor: "pointer",
          }}
        >
          Réessayer
        </button>
      </div>
    );
  }
}
