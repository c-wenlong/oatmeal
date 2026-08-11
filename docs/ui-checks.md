# Checks a unit test cannot make

jsdom has no layout engine. `scrollWidth`, `clientWidth` and `getBoundingClientRect`
all return zero, so **no vitest test can catch truncation, overflow, or a control
pushed off the edge of a window**. Those need a browser at the real size.

The offer window shipped truncated — "Record whe…" over "Asked once. Oatmeal will
r…" — with a full green suite, because every test asserted text that was present
in the DOM and clipped on screen.

## Running them

```
pnpm dev
```

Then, in a browser sized to the window under test:

| what | URL | viewport |
|---|---|---|
| the meeting offer | `/?window=popup` | 520 × 72 |
| the one-time app question | `/?window=popup&popup=question` | 520 × 72 |
| settings, any pane | `/?preview=default` | 1100 × 950 |
| a meeting, recording | `/?preview=default` then open a meeting | 1100 × 950 |

## What to assert

```js
// Nothing is clipped.
[...document.querySelectorAll('.popup-title,.popup-reason')]
  .every(e => e.scrollWidth <= e.clientWidth + 1)

// Nothing overflows the window.
document.body.scrollWidth <= document.body.clientWidth + 1
document.querySelector('.popup').scrollHeight <= 72

// The window can be moved, and no button is also a drag handle.
!!document.querySelector('.popup[data-tauri-drag-region]')
![...document.querySelectorAll('.popup button')]
  .some(b => b.hasAttribute('data-tauri-drag-region'))
```

On the document, also check that the live status has not displaced the page:

```js
// The header is above the live line, not pushed down by it.
const head = document.querySelector('.document-head').getBoundingClientRect()
const live = document.querySelector('.live')?.getBoundingClientRect()
!live || head.top < live.top

// And it is clamped: a long partial must not grow past two lines.
document.querySelector('.live').clientHeight <= 60
```

This is the check that was missing when the live status shipped above
`‹ Meetings`, pushing the title and the summary down the page as the
transcript grew.

The `+ 1` is for sub-pixel rounding, which reports a one-pixel overflow on text
that fits.

## What the unit tests can still do

Guard the *input* to the layout — that a title stays short enough to fit the
space it has, and that both branches carry a drag region. See
`DetectionPopup.test.tsx`. Those catch a regression in the text; only the
browser catches a regression in the box around it.
