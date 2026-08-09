import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TemplatePill, otherTemplates, pillLabel } from "./TemplatePill";
import type { Template } from "../types";

const TEMPLATES: Template[] = [
  { id: "default", name: "Summary", prompt: "" } as Template,
  { id: "one-to-one", name: "1 to 1", prompt: "" } as Template,
  { id: "standup", name: "Stand-up", prompt: "" } as Template,
];

function pill(over: Partial<React.ComponentProps<typeof TemplatePill>> = {}) {
  return (
    <TemplatePill
      templates={TEMPLATES}
      templateId="default"
      busy={false}
      hasPanel={true}
      onGenerate={() => {}}
      {...over}
    />
  );
}

describe("pillLabel", () => {
  it("names the applied template", () => {
    expect(pillLabel(TEMPLATES, "one-to-one", false)).toBe("1 to 1");
  });

  it("says what it is doing while generating", () => {
    expect(pillLabel(TEMPLATES, "default", true)).toBe("Generating…");
  });

  it("falls back rather than rendering undefined for an unknown id", () => {
    // Templates can be deleted while a panel generated from one still exists.
    expect(pillLabel(TEMPLATES, "deleted-template", false)).toBe("Summary");
  });
});

describe("otherTemplates", () => {
  it("omits the one already applied", () => {
    // Offering the applied template twice makes the menu read as a list of
    // options where one is secretly the current state.
    expect(otherTemplates(TEMPLATES, "default").map((t) => t.id)).toEqual([
      "one-to-one",
      "standup",
    ]);
  });

  it("offers everything when none is applied", () => {
    expect(otherTemplates(TEMPLATES, "none")).toHaveLength(3);
  });
});

describe("TemplatePill", () => {
  it("shows the applied template on the pill", () => {
    render(pill({ templateId: "one-to-one" }));
    expect(screen.getByLabelText("template")).toHaveTextContent("1 to 1");
  });

  it("is closed until asked", () => {
    render(pill());
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("regenerates with the applied template from the inline icon", () => {
    const onGenerate = vi.fn();
    render(pill({ templateId: "one-to-one", onGenerate }));
    fireEvent.click(screen.getByLabelText("template"));
    fireEvent.click(screen.getByLabelText("regenerate"));
    expect(onGenerate).toHaveBeenCalledWith("one-to-one");
  });

  it("says generate, not regenerate, when there is no summary yet", () => {
    render(pill({ hasPanel: false }));
    fireEvent.click(screen.getByLabelText("template"));
    expect(screen.getByLabelText("generate")).toBeInTheDocument();
  });

  it("generates with a different template when one is chosen", () => {
    const onGenerate = vi.fn();
    render(pill({ onGenerate }));
    fireEvent.click(screen.getByLabelText("template"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Stand-up" }));
    expect(onGenerate).toHaveBeenCalledWith("standup");
  });

  it("closes after choosing", () => {
    render(pill());
    fireEvent.click(screen.getByLabelText("template"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Stand-up" }));
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("cannot be opened while generating", () => {
    // Choosing a second template mid-generation would queue a request whose
    // result silently replaces the one being waited for.
    render(pill({ busy: true }));
    expect(screen.getByLabelText("template")).toBeDisabled();
  });

  it("keeps provider and model out of the document, in the menu", () => {
    render(pill({ footer: <span>ollama · gemma4:e2b</span> }));
    expect(screen.queryByText(/gemma4/)).toBeNull();

    fireEvent.click(screen.getByLabelText("template"));
    expect(screen.getByText(/gemma4/)).toBeInTheDocument();
  });

  it("closes on Escape and on a click elsewhere", () => {
    render(pill());
    fireEvent.click(screen.getByLabelText("template"));
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();

    fireEvent.click(screen.getByLabelText("template"));
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("menu")).toBeNull();
  });
});
