import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { LoadingState } from "./LoadingState";
import { KeyRound } from "lucide-react";

describe("LoadingState", () => {
  it("renders loading spinner", () => {
    const { container } = render(<LoadingState />);
    const spinner = container.querySelector(".ui-loading-spinner");
    expect(spinner).toBeInTheDocument();
  });

  it("renders message when provided", () => {
    render(<LoadingState message="Loading data..." />);
    expect(screen.getByText("Loading data...")).toBeInTheDocument();
  });

  it("renders icon when provided", () => {
    const { container } = render(<LoadingState icon={KeyRound} />);
    const icon = container.querySelector(".ui-loading-icon");
    expect(icon).toBeInTheDocument();
  });

  it("renders without message or icon", () => {
    const { container } = render(<LoadingState />);
    expect(container.querySelector(".ui-loading-message")).not.toBeInTheDocument();
    expect(container.querySelector(".ui-loading-icon")).not.toBeInTheDocument();
  });
});
