import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ErrorState } from "./ErrorState";

describe("ErrorState", () => {
  it("renders title", () => {
    render(<ErrorState title="Error occurred" />);
    expect(screen.getByText("Error occurred")).toBeInTheDocument();
  });

  it("has alert role", () => {
    render(<ErrorState title="Error occurred" />);
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  it("renders description when provided", () => {
    render(<ErrorState title="Error" description="Something went wrong" />);
    expect(screen.getByText("Something went wrong")).toBeInTheDocument();
  });

  it("renders action when provided", () => {
    const handleRetry = vi.fn();
    render(
      <ErrorState
        title="Error"
        action={<button onClick={handleRetry}>Retry</button>}
      />,
    );
    const button = screen.getByText("Retry");
    fireEvent.click(button);
    expect(handleRetry).toHaveBeenCalledTimes(1);
  });

  it("does not render description or action when omitted", () => {
    const { container } = render(<ErrorState title="Error" />);
    expect(container.querySelector(".ui-error-state-desc")).not.toBeInTheDocument();
    expect(container.querySelector(".ui-error-state-action")).not.toBeInTheDocument();
  });
});
