import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { OverflowMenu } from "./OverflowMenu";

describe("OverflowMenu", () => {
  it("shows nothing until asked", () => {
    render(<OverflowMenu items={[{ label: "Settings", onSelect: () => {} }]} />);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("opens on click", () => {
    render(<OverflowMenu items={[{ label: "Settings", onSelect: () => {} }]} />);
    fireEvent.click(screen.getByLabelText("more"));
    expect(screen.getByRole("menuitem", { name: "Settings" })).toBeInTheDocument();
  });

  it("runs the item that was chosen", () => {
    const chosen = vi.fn();
    const other = vi.fn();
    render(
      <OverflowMenu
        items={[
          { label: "Settings", onSelect: chosen },
          { label: "Workbench", onSelect: other },
        ]}
      />,
    );
    fireEvent.click(screen.getByLabelText("more"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Settings" }));

    expect(chosen).toHaveBeenCalled();
    expect(other).not.toHaveBeenCalled();
  });

  it("closes after choosing, so it does not float over the next screen", () => {
    render(<OverflowMenu items={[{ label: "Settings", onSelect: () => {} }]} />);
    fireEvent.click(screen.getByLabelText("more"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Settings" }));
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("closes on a click elsewhere", () => {
    // Otherwise dismissing it is a deliberate act, which is one interaction
    // more than a menu is worth.
    render(<OverflowMenu items={[{ label: "Settings", onSelect: () => {} }]} />);
    fireEvent.click(screen.getByLabelText("more"));
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("closes on Escape", () => {
    render(<OverflowMenu items={[{ label: "Settings", onSelect: () => {} }]} />);
    fireEvent.click(screen.getByLabelText("more"));
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("stays open when the click is inside it", () => {
    render(
      <OverflowMenu
        items={[
          { label: "Settings", onSelect: () => {} },
          { label: "Workbench", onSelect: () => {} },
        ]}
      />,
    );
    const button = screen.getByLabelText("more");
    fireEvent.click(button);
    fireEvent.mouseDown(button);
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });

  it("reports its state to assistive technology", () => {
    render(<OverflowMenu items={[{ label: "Settings", onSelect: () => {} }]} />);
    const button = screen.getByLabelText("more");
    expect(button).toHaveAttribute("aria-expanded", "false");
    fireEvent.click(button);
    expect(button).toHaveAttribute("aria-expanded", "true");
  });
});
