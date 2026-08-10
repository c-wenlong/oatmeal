import { useCallback, useEffect, useState } from "react";
import { updateCheck, updateInstall, updateSkip } from "../lib/tauri";
import type { UpdateStatus } from "../types";

/** What to say about the current state, in one line. */
export function summarise(status: UpdateStatus): string {
  switch (status.decision) {
    case "offer":
      return `Version ${status.availableVersion} is available.`;
    case "skipped":
      return `Version ${status.availableVersion} is available — you chose to skip it.`;
    case "up_to_date":
      return `Oatmeal ${status.currentVersion} is up to date.`;
  }
}

/**
 * A failure in the user's terms.
 *
 * The updater's own wording — "Could not fetch a valid release JSON from the
 * remote" — is accurate and tells you nothing you can act on. On a project with
 * no releases yet it means precisely one thing, and even where it does not, the
 * two possibilities are worth naming.
 */
export function describeFailure(error: string): string {
  if (/release json|manifest/i.test(error)) {
    return "No release manifest at that address — either nothing has been published yet, or the manifest is malformed.";
  }
  if (/not configured|pubkey|public key/i.test(error)) {
    return `Updates are not configured, so none can be installed. ${error}`;
  }
  return error;
}

/**
 * Whether a failed check is worth showing the user.
 *
 * A laptop on a train fails this check every time, and an update check is
 * never a good reason to interrupt someone with an error. A missing public
 * key is different: it means updates can never work at all, and silence would
 * hide that until the day a security fix needed shipping.
 */
export function isWorthReporting(error: string): boolean {
  return /not configured|pubkey|public key|signature/i.test(error);
}

/**
 * In-place updates.
 *
 * The app asks GitHub rather than GitHub telling the app — an installed copy
 * has no address to be called at. So this checks once on mount, which is app
 * launch in practice, and otherwise stays out of the way.
 *
 * Nothing installs without a click. An app that swaps itself out mid-meeting
 * and restarts would lose a recording, which is the one thing this product
 * cannot do.
 */
export function UpdateCard() {
  const [status, setStatus] = useState<UpdateStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  const check = useCallback(async (announce: boolean) => {
    setBusy(true);
    if (announce) setMessage("Checking…");
    try {
      const next = await updateCheck();
      setStatus(next);
      setMessage(null);
    } catch (err) {
      const text = String(err);
      // On an explicit click, say what happened either way — silence after a
      // button press reads as a broken button.
      setMessage(announce || isWorthReporting(text) ? describeFailure(text) : null);
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void check(false);
  }, [check]);

  async function install() {
    setBusy(true);
    setMessage("Downloading… Oatmeal will restart when it finishes.");
    try {
      await updateInstall();
    } catch (err) {
      setMessage(String(err));
      setBusy(false);
    }
  }

  async function skip() {
    if (!status?.availableVersion) return;
    await updateSkip(status.availableVersion);
    await check(false);
  }

  return (
    <section className="card" data-testid="update-card">
      <div className="card-head">
        <h2>Updates</h2>
        {status?.decision === "offer" && <span className="pill">available</span>}
      </div>

      <p className="card-note">
        {status ? summarise(status) : "Checking for updates…"}
      </p>

      {status?.notes && status.decision !== "up_to_date" && (
        <p className="empty-note">{status.notes}</p>
      )}

      <div className="row">
        {status && status.decision !== "up_to_date" ? (
          <>
            <button className="primary" disabled={busy} onClick={() => void install()}>
              {busy ? "Working…" : "Install and restart"}
            </button>
            {status.decision === "offer" && (
              <button disabled={busy} onClick={() => void skip()}>
                Skip this version
              </button>
            )}
          </>
        ) : (
          <button disabled={busy} onClick={() => void check(true)}>
            {busy ? "Checking…" : "Check for updates"}
          </button>
        )}
      </div>

      {message && <p className="empty-note">{message}</p>}
    </section>
  );
}
