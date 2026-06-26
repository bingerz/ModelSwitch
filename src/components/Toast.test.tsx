import { vi, describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { ToastProvider, useToast } from "./Toast";

function TestConsumer() {
  const { success, error, info } = useToast();
  return (
    <div>
      <span>children-content</span>
      <button onClick={() => success("Done!")}>success-btn</button>
      <button onClick={() => error("Failed!")}>error-btn</button>
      <button onClick={() => info("FYI")}>info-btn</button>
    </div>
  );
}

describe("ToastProvider", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders_children", () => {
    render(
      <ToastProvider>
        <TestConsumer />
      </ToastProvider>,
    );
    expect(screen.getByText("children-content")).toBeTruthy();
  });

  it("success_toast_appears_when_triggered", () => {
    render(
      <ToastProvider>
        <TestConsumer />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByText("success-btn"));
    expect(screen.getByText("Done!")).toBeTruthy();
  });

  it("error_toast_appears_when_triggered", () => {
    render(
      <ToastProvider>
        <TestConsumer />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByText("error-btn"));
    expect(screen.getByText("Failed!")).toBeTruthy();
  });

  it("info_toast_appears_when_triggered", () => {
    render(
      <ToastProvider>
        <TestConsumer />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByText("info-btn"));
    expect(screen.getByText("FYI")).toBeTruthy();
  });

  it("toast_has_correct_type_class", () => {
    const { container } = render(
      <ToastProvider>
        <TestConsumer />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByText("success-btn"));
    expect(container.querySelector(".toast-success")).not.toBeNull();
  });

  it("toast_auto_dismisses_after_3_seconds", () => {
    render(
      <ToastProvider>
        <TestConsumer />
      </ToastProvider>,
    );
    fireEvent.click(screen.getByText("success-btn"));
    expect(screen.getByText("Done!")).toBeTruthy();

    act(() => {
      vi.advanceTimersByTime(3000);
    });

    expect(screen.queryByText("Done!")).toBeNull();
  });
});
