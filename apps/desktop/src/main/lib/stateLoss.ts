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
 *
 * WHY AN EMPTY SUBDIRECTORY IS NOT EVIDENCE
 *
 * Counting any entry meant counting a directory somebody's installer had just
 * made. On the Fly node, `deploy/fly/node/bootstrap.sh` pre-creates `plugins/`,
 * `library/`, `layouts/`, `sessions/` and `logs/` inside `<config>/workspacer`
 * before the brain starts, so the brain reported STATE LOSS on every
 * genuinely-first boot. A guard that is wrong on every first boot is one the
 * operator learns to scroll past, which costs it the cases it exists for. So an
 * entry counts only when it holds something. A bare mkdir proves a mkdir ran; it
 * does not prove the program ran.
 */
import * as fs from 'fs';
import * as path from 'path';

/**
 * True when `name` is missing from `dir` in a way that looks like loss rather
 * than a first run: the directory exists and still holds at least one entry,
 * other than `name` itself, that carries actual state.
 *
 * An entry carries state if it is anything but an empty directory.
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
  return entries.some((e) => e !== name && !holdsNothing(path.join(dir, e)));
}

/**
 * True only for a directory that holds nothing at all.
 *
 * Everything else is false, and each case is deliberate. A regular file makes
 * `readdirSync` throw ENOTDIR, so a file always counts as evidence, including a
 * zero-byte one, because something wrote it. A directory that cannot be read
 * throws too, and unreadable is unknown: the safe answer for a guard biased
 * toward the loud outcome is to let it count.
 *
 * Written with `readdirSync` rather than `statSync` or `withFileTypes` on
 * purpose. One call answers both questions at once, and it keeps this function
 * working against the plain name-list that every `fs` test double returns.
 */
function holdsNothing(entry: string): boolean {
  try {
    return fs.readdirSync(entry).length === 0;
  } catch {
    return false;
  }
}
