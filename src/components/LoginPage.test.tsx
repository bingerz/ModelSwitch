import { vi } from "vitest";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, params?: Record<string, unknown>) =>
      params
        ? Object.entries(params).reduce(
            (s, [k, v]) => s.replace(`{${k}}`, String(v)),
            key,
          )
        : key,
    i18n: { language: "en", changeLanguage: vi.fn() },
  }),
}));

vi.mock("../lib/api", () => ({
  request: vi.fn(),
}));

import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { LoginPage } from "./LoginPage";
import { request } from "../lib/api";

const mockedRequest = vi.mocked(request);

/** Node 26 ships an experimental localStorage global that shadows jsdom's.
 *  Provide a minimal in-memory Storage so component code works. */
function createStorage(): Storage {
  let store: Record<string, string> = {};
  return {
    get length() {
      return Object.keys(store).length;
    },
    getItem: (key: string) => store[key] ?? null,
    setItem: (key: string, value: string) => {
      store[key] = value;
    },
    removeItem: (key: string) => {
      delete store[key];
    },
    clear: () => {
      store = {};
    },
    key: (index: number) => Object.keys(store)[index] ?? null,
  };
}

beforeEach(() => {
  vi.stubGlobal("localStorage", createStorage());
  vi.stubGlobal("sessionStorage", createStorage());
});

describe("LoginPage", () => {
  beforeEach(() => {
    localStorage.clear();
    sessionStorage.clear();
    mockedRequest.mockReset();
  });

  afterEach(() => {
    localStorage.clear();
    sessionStorage.clear();
  });

  it("renders_brand_and_input", () => {
    render(<LoginPage onSuccess={() => {}} />);
    expect(screen.getByText("login.brand")).toBeTruthy();
    expect(screen.getByPlaceholderText("login.tokenPlaceholder")).toBeTruthy();
    expect(screen.getByText("login.button")).toBeTruthy();
  });

  it("shows_session_expired_message_when_flag_set", () => {
    sessionStorage.setItem("auth_expired", "1");
    render(<LoginPage onSuccess={() => {}} />);
    expect(screen.getByText("login.sessionExpired")).toBeTruthy();
  });

  it("does_not_show_expired_message_by_default", () => {
    render(<LoginPage onSuccess={() => {}} />);
    expect(screen.queryByText("login.sessionExpired")).toBeNull();
  });

  it("calls_onSuccess_on_valid_login", async () => {
    mockedRequest.mockResolvedValue(undefined);
    const onSuccess = vi.fn();
    render(<LoginPage onSuccess={onSuccess} />);

    const input = screen.getByPlaceholderText("login.tokenPlaceholder");
    fireEvent.change(input, { target: { value: "test-token" } });
    fireEvent.click(screen.getByText("login.button"));

    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledTimes(1);
    });
    expect(mockedRequest).toHaveBeenCalledWith("/api/gateway/info");
  });

  it("shows_error_on_login_failure", async () => {
    mockedRequest.mockRejectedValue(new Error("Unauthorized"));
    render(<LoginPage onSuccess={() => {}} />);

    fireEvent.click(screen.getByText("login.button"));

    await waitFor(() => {
      expect(screen.getByText("login.error")).toBeTruthy();
    });
  });

  it("shows_network_error_on_fetch_type_error", async () => {
    mockedRequest.mockRejectedValue(new TypeError("fetch error"));
    render(<LoginPage onSuccess={() => {}} />);

    fireEvent.click(screen.getByText("login.button"));

    await waitFor(() => {
      expect(screen.getByText("login.networkError")).toBeTruthy();
    });
  });

  it("stores_token_in_localStorage_on_login", async () => {
    mockedRequest.mockResolvedValue(undefined);
    render(<LoginPage onSuccess={() => {}} />);

    const input = screen.getByPlaceholderText("login.tokenPlaceholder");
    fireEvent.change(input, { target: { value: "my-secret-token" } });
    fireEvent.click(screen.getByText("login.button"));

    await waitFor(() => {
      expect(localStorage.getItem("admin_token")).toBe("my-secret-token");
    });
  });

  it("renders_admin_portal_mode_explanation", () => {
    render(<LoginPage onSuccess={() => {}} />);
    expect(screen.getByText("login.modeTitle")).toBeTruthy();
    expect(screen.getByText("login.modeExplanation")).toBeTruthy();
    // Mode labels appear in <strong> tags and repeated in containing <li>
    // Use getAllByText to handle multiple matches
    expect(screen.getAllByText(/login\.adminMode/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/login\.adminModeDesc/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/login\.portalMode/).length).toBeGreaterThan(0);
    expect(screen.getAllByText(/login\.portalModeDesc/).length).toBeGreaterThan(0);
  });
});
