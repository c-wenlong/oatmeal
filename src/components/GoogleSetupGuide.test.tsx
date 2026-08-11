import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { GUIDE_STEPS, GoogleSetupGuide, hasScreenshot } from "./GoogleSetupGuide";

describe("hasScreenshot", () => {
  it("needs both the image and its alt text", () => {
    // An image with no alt text is a screenshot that says nothing to anyone
    // who cannot see it, so a half-filled entry degrades to text.
    expect(hasScreenshot({ title: "t", body: "b" })).toBe(false);
    expect(hasScreenshot({ title: "t", body: "b", image: "guide/a.png" })).toBe(false);
    expect(
      hasScreenshot({ title: "t", body: "b", image: "guide/a.png", alt: "A" }),
    ).toBe(true);
  });
});

describe("GUIDE_STEPS", () => {
  it("covers the four things that actually go wrong", () => {
    // Each of these cost real time to discover: the API left disabled, the
    // missing test user, the Web-application client that cannot use a loopback
    // port, and the secret Google requires despite PKCE.
    const all = GUIDE_STEPS.map((s) => `${s.title} ${s.body}`).join(" ");
    expect(all).toMatch(/enable it/i);
    expect(all).toMatch(/Test users/i);
    expect(all).toMatch(/Not Web application/i);
    expect(all).toMatch(/requires the secret/i);
  });

  it("names a picture only alongside its alt text", () => {
    for (const step of GUIDE_STEPS) {
      if (step.image)
        expect(step.alt, `${step.title} has an image but no alt`).toBeTruthy();
    }
  });
});

describe("GoogleSetupGuide", () => {
  it("renders every step, numbered", () => {
    render(<GoogleSetupGuide onBack={() => {}} />);
    for (const step of GUIDE_STEPS) {
      expect(screen.getByText(step.title)).toBeInTheDocument();
    }
  });

  it("reads as text before any screenshot exists", () => {
    // The guide has to be useful the day it is written, not the day the
    // pictures are added.
    render(<GoogleSetupGuide onBack={() => {}} />);
    expect(document.querySelectorAll("img")).toHaveLength(
      GUIDE_STEPS.filter(hasScreenshot).length,
    );
  });

  it("goes back to the screen it was opened from", () => {
    // A level higher would lose the place: the guide is read while setting up
    // the Google screen.
    const onBack = vi.fn();
    render(<GoogleSetupGuide onBack={onBack} />);
    fireEvent.click(screen.getByRole("button", { name: /Google Calendar/ }));
    expect(onBack).toHaveBeenCalled();
  });
});
