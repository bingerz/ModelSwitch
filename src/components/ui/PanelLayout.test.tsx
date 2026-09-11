import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { PanelLayout } from "./PanelLayout";
import { Settings } from "lucide-react";

// Mock react-i18next
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
  }),
}));

describe("PanelLayout", () => {
  it("renders title", () => {
    render(
      <PanelLayout title="Test Panel">
        <div>Content</div>
      </PanelLayout>
    );
    expect(screen.getByText("Test Panel")).toBeInTheDocument();
  });

  it("renders icon when provided", () => {
    const { container } = render(
      <PanelLayout title="Test" icon={Settings}>
        <div>Content</div>
      </PanelLayout>
    );
    expect(container.querySelector(".panel-layout-icon")).toBeInTheDocument();
  });

  it("renders children in content area", () => {
    render(
      <PanelLayout title="Test">
        <div data-testid="test-content">Content</div>
      </PanelLayout>
    );
    expect(screen.getByTestId("test-content")).toBeInTheDocument();
  });

  it("renders actions when provided", () => {
    render(
      <PanelLayout
        title="Test"
        actions={<button>Action</button>}
      >
        <div>Content</div>
      </PanelLayout>
    );
    expect(screen.getByText("Action")).toBeInTheDocument();
  });

  it("renders toolbar when provided", () => {
    render(
      <PanelLayout
        title="Test"
        toolbar={<div data-testid="toolbar">Toolbar</div>}
      >
        <div>Content</div>
      </PanelLayout>
    );
    expect(screen.getByTestId("toolbar")).toBeInTheDocument();
  });

  it("renders refresh button when onRefresh provided", () => {
    const handleRefresh = vi.fn();
    render(
      <PanelLayout title="Test" onRefresh={handleRefresh}>
        <div>Content</div>
      </PanelLayout>
    );
    const refreshButton = screen.getByTitle("common.refreshNow");
    expect(refreshButton).toBeInTheDocument();
    fireEvent.click(refreshButton);
    expect(handleRefresh).toHaveBeenCalledTimes(1);
  });

  it("disables refresh button when refreshing", () => {
    render(
      <PanelLayout
        title="Test"
        onRefresh={() => {}}
        refreshing={true}
      >
        <div>Content</div>
      </PanelLayout>
    );
    const refreshButton = screen.getByTitle("common.refreshNow");
    expect(refreshButton).toBeDisabled();
  });

  it("does not render refresh button when onRefresh not provided", () => {
    render(
      <PanelLayout title="Test">
        <div>Content</div>
      </PanelLayout>
    );
    expect(screen.queryByTitle("common.refreshNow")).not.toBeInTheDocument();
  });
});
