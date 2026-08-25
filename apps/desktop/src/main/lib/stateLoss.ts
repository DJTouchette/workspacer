/**
 * Tells a genuine FIRST RUN apart from state that has VANISHED.
 *
 * The TypeScript twin of `services/hub/internal/statelost` (Go), which carries
 * the long form of this reasoning. Both halves need it because both halves mint
 * the same file: `<config>/workspacer/remote-token` is written by the desktop's
 * hubDaemon.loadOrCreateToken AND by the CLI's token.go, deliberately, so a
 * phone paired against one keeps working against the other.
 *
 * Both were shaped "read the file; if it is not there, make a new one", which is
 * right the first time and wrong every time after: a recreated pairing
 * credential is a DIFFERENT credential, so the app comes up looking healthy
 * while refusing every client, phone and federation peer that knew the old one.
 * Nothing logged anything, because from the loader's point of view nothing went
 * wrong.
 *
 * The directory around the missing file is the evidence. Empty means nobody has
 * ever run here. Still holding the rest of the state means something took this
 * one file away.
 */
import * as fs from 'fs';

/**
 * True when `name` is missing from `dir` in a way that looks like loss rather
 * than a first run: the directory exists and still holds at least one entry that
 * is not `name` itself.
 *
 * A directory that cannot be read at all (including one that does not exist) is
 * reported as NOT lost — nobody has ever run there, so there is nothing to have
 * lost.
 *
 * Deliberately coarse, and the asymmetry is the point: a false positive is a
 * loud message, a false negative is the silent failure this exists to end.
 */
export function suspectedStateLoss(dir: string, name: string): boolean {
  let entries: string[];
  try {
    entries = fs.readdirSync(dir);
  } catch {
    return false;
  }
  return entries.some((e) => e !== name);
}
