Screenshots for the Google setup guide.

Drop a PNG here, then name it on its step in
`src/components/GoogleSetupGuide.tsx`:

    { title: "…", body: "…", image: "guide/create-client.png", alt: "…" }

`alt` is required alongside `image` — a step renders its picture only when
both are present, so a screenshot is never mute to someone who cannot see it,
and a half-finished entry degrades to text rather than to a broken image.

Paths are relative to the app root, so `public/guide/x.png` is `guide/x.png`.
