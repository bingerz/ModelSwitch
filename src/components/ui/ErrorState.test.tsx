import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ErrorState } from "./ErrorState";
import { AlertCircle } from "lucide-react";

describe("ErrorState", () => {
  it("renders title and message", () => {
    render(
      <ErrorState
        title="Error occurred"
        message="Something went wrong"
      />
    );
    expect(screen.getByText("Error occurred")).toBeInTheDocument();
    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
  });

  it("renders custom icon", () => {
    const { container } = render(
      <ErrorState
        icon={AlertCircle}
        title="Error"
        message="Test"
      />
    );
    expect(container.querySelector(".ui-error-icon")).toBeInTheDocument();
  });

  it("renders retry button when onRetry provided", () => {
    const handleRetry = vi.fn();
    render(
      <ErrorState
        title="Error"
        message="Test"
        onRetry={handleRetry}
      />
    );
    const retryButton = screen.getByText("Retry");
    expect(retryButton).toBeInTheDocument();
    fireEvent.click(retryButton);
    expect(handleRetry).toHaveBeenCalledTimes(1);
  });

  it("uses custom retry label", () => {
    render(
      <ErrorState
        title="Error"
        message="Test"
        onRetry={() => {}}
        retryLabel="Try Again"
      />
    );
    expect(screen.getByText("Try Again")).toBeInTheDocument();
  });

  it("does not render retry button when onRetry not provided", () => {
    render(
      <ErrorState
        title="Error"
        message="Test"
      />
    );
    expect(screen.queryByText("Retry")).not.toBeInTheDocument();
  });
});
