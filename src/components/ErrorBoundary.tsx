import { Component, type ErrorInfo, type ReactNode } from "react";
import { withTranslation, type WithTranslation } from "react-i18next";

/**
 * Optional custom fallback. When provided, the boundary renders this instead of
 * the default full-screen error card. Useful for per-panel isolation where a
 * single panel failure should not take over the entire viewport.
 *
 * The render-prop form receives the caught error so callers can display
 * targeted diagnostics.
 */
type ErrorBoundaryFallback = ReactNode | ((error: Error) => ReactNode);

interface ErrorBoundaryProps extends WithTranslation {
  children: ReactNode;
  fallback?: ErrorBoundaryFallback;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null };
  }

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error("[ErrorBoundary] Caught render error:", error);
    console.error("[ErrorBoundary] Component stack:", info.componentStack);
  }

  render() {
    if (this.state.hasError) {
      const { fallback } = this.props;
      if (fallback !== undefined) {
        return typeof fallback === "function"
          ? fallback(this.state.error ?? new Error("Unknown error"))
          : fallback;
      }

      const { t } = this.props;
      return (
        <div className="error-boundary-overlay">
          <div className="error-boundary-card">
            <div className="error-boundary-icon">!</div>
            <h1 className="error-boundary-title">{t("error.boundaryTitle")}</h1>
            <p className="error-boundary-message">
              {this.state.error?.message ?? t("error.boundaryMessage")}
            </p>
            <button
              className="btn btn-primary"
              onClick={() => window.location.reload()}
            >
              {t("error.reloadApp")}
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

export default withTranslation()(ErrorBoundary);
