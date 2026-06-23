import { Component, type ErrorInfo, type ReactNode } from "react";
import { withTranslation, type WithTranslation } from "react-i18next";

interface ErrorBoundaryProps extends WithTranslation {
  children: ReactNode;
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
