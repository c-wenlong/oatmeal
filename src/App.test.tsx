import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import App, { isPopupWindow } from "./App";

// The library screen is four components deep in Tauri calls. These tests are
// about which screen the window is showing, so each stands in as a marker.
vi.mock("./components/Library", () => ({ Library: () => <div /> }));
vi.mock("./components/Onboarding", () => ({ Onboarding: () => <div /> }));
vi.mock("./components/NewMeetingButton", () => ({ NewMeetingButton: () => <div /> }));
vi.mock("./components/AskBar", () => ({ AskBar: () => <div /> }));
vi.mock("./components/MeetingDocument", () => ({ MeetingDocument: () => <div /> }));
vi.mock("./components/DetectionPopup", () => ({
  DetectionPopup: () => <div data-testid="popup" />,
}));
vi.mock("./components/Settings", () => ({
  Settings: () => <div data-testid="settings" />,
}));

describe("isPopupWindow", () => {
  it("trusts the window label over the query string", () => {
    // Tauri assigns the label at creation and it survives a reload that a
    // query string would not.
    expect(isPopupWindow("", "popup")).toBe(true);
    expect(isPopupWindow("?window=popup", "main")).toBe(false);
  });

  it("falls back to the query string when there is no label", () => {
    expect(isPopupWindow("?window=popup")).toBe(true);
    expect(isPopupWindow("")).toBe(false);
  });
});

describe("⌘, ", () => {
  it("opens settings from the library", () => {
    render(<App />);
    expect(screen.queryByTestId("settings")).toBeNull();
    fireEvent.keyDown(document, { key: ",", metaKey: true });
    expect(screen.getByTestId("settings")).toBeInTheDocument();
  });

  it("does not open on a bare comma", () => {
    // Typing a comma anywhere in the app must stay a comma.
    render(<App />);
    fireEvent.keyDown(document, { key: "," });
    expect(screen.queryByTestId("settings")).toBeNull();
  });

  it("stops the webview acting on the key as well", () => {
    render(<App />);
    // fireEvent returns false when the event was cancelled — WebKit has its
    // own idea about ⌘, and both of them firing is one too many.
    const uncancelled = fireEvent.keyDown(document, { key: ",", metaKey: true });
    expect(uncancelled).toBe(false);
  });

  it("is not listening in the detection popup", () => {
    // A second window, a different job, and no settings screen behind it.
    const search = window.location.search;
    Object.defineProperty(window, "location", {
      value: { ...window.location, search: "?window=popup" },
      writable: true,
    });
    try {
      render(<App />);
      expect(screen.getByTestId("popup")).toBeInTheDocument();
      fireEvent.keyDown(document, { key: ",", metaKey: true });
      expect(screen.queryByTestId("settings")).toBeNull();
    } finally {
      Object.defineProperty(window, "location", {
        value: { ...window.location, search },
        writable: true,
      });
    }
  });
});
