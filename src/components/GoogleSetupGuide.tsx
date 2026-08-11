/**
 * How to get a Desktop app OAuth client, step by step.
 *
 * Its own screen rather than a paragraph on the card. The card asks for two
 * long opaque strings, and the instructions for producing them run to a dozen
 * clicks across a console most people have never opened — that is a guide, and
 * a guide squeezed into a settings row is one nobody follows to the end.
 *
 * Each step carries an optional `image`. Drop a screenshot into
 * `public/guide/` and name it here; a step with no image renders as text, so
 * the guide is useful before a single picture exists and improves as they are
 * added, rather than being broken until all of them are.
 */

export interface GuideStep {
  title: string;
  /** What to do. One action per step — a step with two verbs gets half done. */
  body: string;
  /** A file in `public/guide/`, e.g. `"guide/create-client.png"`. */
  image?: string;
  /** Alt text. Required alongside an image, so a screenshot is never mute. */
  alt?: string;
}

export const GUIDE_STEPS: GuideStep[] = [
  {
    title: "Create a Google Cloud project",
    body: "Open console.cloud.google.com and make a project, or pick one you already have. It is free, and nothing runs in it — Oatmeal talks to Google directly from your Mac.",
  },
  {
    title: "Enable the Google Calendar API",
    body: "In that project, go to APIs & Services › Library, search for Google Calendar API, and enable it. Without this the credentials exist but every request is refused.",
  },
  {
    title: "Configure the consent screen",
    body: "Under APIs & Services › OAuth consent screen, choose External, fill in the app name and your email, and add your own Google account under Test users. Skipping the test user is why a correct client still ends in access_denied.",
  },
  {
    title: "Create the credential — Desktop app",
    body: "Credentials › Create credentials › OAuth client ID, and choose Desktop app as the type. Not Web application: Oatmeal listens on a loopback port that changes every attempt, and only the Desktop type accepts that without registering each one.",
  },
  {
    title: "Copy both halves into Oatmeal",
    body: "Google shows a client ID and a client secret. Paste both into the Google Calendar screen. Google requires the secret for Desktop app clients even though Oatmeal uses PKCE — it documents the secret as non-confidential for installed apps, which is not the same as optional. The ID is not secret; the secret goes to your Keychain and is never written to a file.",
  },
  {
    title: "Connect",
    body: "Turn on the switch at the top of the Google Calendar screen. Your browser opens, Google asks you to approve read-only access to your events and the names of your calendars, and the calendars appear in Settings › Calendar.",
  },
];

/** Whether a step is ready to be shown with a picture. */
export function hasScreenshot(step: GuideStep): boolean {
  // Both or neither: an image with no alt text is a screenshot that says
  // nothing to anyone who cannot see it.
  return Boolean(step.image && step.alt);
}

export function GoogleSetupGuide({ onBack }: { onBack: () => void }) {
  return (
    <div data-testid="google-setup-guide">
      <button className="document-back" onClick={onBack}>
        ‹ Google Calendar
      </button>
      <h1 className="library-title settings-title">Getting a Desktop app client</h1>

      <p className="card-note guide-intro">
        Oatmeal talks to Google with your own credential rather than one shipped inside
        the app — a secret in a downloadable binary is not a secret. It takes about five
        minutes, once.
      </p>

      <ol className="guide">
        {GUIDE_STEPS.map((step, index) => (
          <li className="guide-step" key={step.title}>
            <span className="guide-number" aria-hidden="true">
              {index + 1}
            </span>
            <div className="guide-body">
              <h2 className="guide-title">{step.title}</h2>
              <p className="guide-text">{step.body}</p>
              {hasScreenshot(step) && (
                <img className="guide-shot" src={step.image} alt={step.alt} />
              )}
            </div>
          </li>
        ))}
      </ol>
    </div>
  );
}
