import { Component, type ReactNode } from "react";
import { AlertTriangle } from "lucide-react";

interface Props {
  children: ReactNode;
  fallback?: ReactNode;
  onError?: (error: Error, errorInfo: React.ErrorInfo) => void;
}

interface State {
  hasError: boolean;
  error?: Error;
}

export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error("ErrorBoundary caught error:", error, errorInfo);
    this.props.onError?.(error, errorInfo);
  }

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) {
        return this.props.fallback;
      }
      return (
        <div className="flex items-start gap-3 border border-red-500/40 bg-red-950/30 p-3 text-sm">
          <AlertTriangle className="h-5 w-5 flex-shrink-0 text-red-300" />
          <div>
            <div className="font-medium">Component Error</div>
            <div className="text-red-100">{this.state.error?.message ?? "Unknown error"}</div>
            <button
              className="mt-2 border border-red-500/40 px-2 py-1 text-xs hover:bg-red-950/50"
              onClick={() => this.setState({ hasError: false, error: undefined })}
            >
              Reset
            </button>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}
