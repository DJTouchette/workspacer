import { describe, it, expect } from 'vitest';
import { readFileSync } from 'fs';
import { join } from 'path';
import {
  EMPTY_DRAFT,
  POWER_DOWN_PLACEHOLDER,
  TEMPLATES,
  draftProblem,
  hasUnfilledScript,
  toJob,
} from '../src/components/settings/JobsSection';
import type { Draft } from '../src/components/settings/JobsSection';

/**
 * The power-down-when-quiet job exists in two places: the documented example in
 * landing/docs.html, and the template chip in Settings -> Jobs that pre-fills
 * the editor with it. contracts/job-preset-power-down.json is the one thing
 * both are held to, so a change to either has to move the fixture and fail the
 * other side until it follows.
 *
 * This is the renderer half. The Go half is
 * services/hub/internal/jobs/preset_test.go, which runs the same fixture
 * through the REAL Validate the hub applies on save and checks it against the
 * documented block.
 */

const REPO_ROOT = join(__dirname, '..', '..', '..', '..', '..');
const FIXTURE = JSON.parse(
  readFileSync(join(REPO_ROOT, 'contracts', 'job-preset-power-down.json'), 'utf8'),
) as {
  quiescenceCheck: string;
  placeholder: string;
  spec: Record<string, unknown>;
};

function powerDownDraft(): Draft {
  const tpl = TEMPLATES.find((t) => t.draft.command?.includes(POWER_DOWN_PLACEHOLDER));
  if (!tpl) throw new Error('no template pre-fills the power-down command');
  return { ...EMPTY_DRAFT, ...tpl.draft };
}

describe('contracts/job-preset-power-down.json', () => {
  it('the template produces exactly the fixture spec', () => {
    const d = powerDownDraft();
    const job = toJob(d, d.enabledOnCreate);
    // JSON round trip so the undefined optionals toJob leaves behind (cwd,
    // context, model) compare the way the wire sees them.
    expect(JSON.parse(JSON.stringify(job))).toEqual(FIXTURE.spec);
  });

  it('the placeholder in the template is the fixture placeholder', () => {
    expect(POWER_DOWN_PLACEHOLDER).toBe(FIXTURE.placeholder);
  });

  it('the command runs the quiescence check the fixture names, and stops there', () => {
    const d = powerDownDraft();
    const [check, script] = d.command.split(' && ');
    expect(check).toBe(FIXTURE.quiescenceCheck);
    // The blank is the whole point: a plausible path here is one someone saves
    // without reading, and no machine has /opt/wks/power-down.sh on it.
    expect(script).toBe(POWER_DOWN_PLACEHOLDER);
  });
});

describe('the power-down template arrives disarmed', () => {
  it('does not enable the job on Create', () => {
    expect(powerDownDraft().enabledOnCreate).toBe(false);
    // Every other template still arms what it creates.
    for (const tpl of TEMPLATES) {
      if (tpl.draft.command?.includes(POWER_DOWN_PLACEHOLDER)) continue;
      expect({ ...EMPTY_DRAFT, ...tpl.draft }.enabledOnCreate).toBe(true);
    }
  });

  it('is still savable, because a disabled job cannot run', () => {
    // The safety comes from enabled:false, not from refusing to save. Someone
    // who wants to fill the path in later should be able to.
    expect(draftProblem(powerDownDraft())).toBeNull();
  });

  it('flags the unfilled command, and stops flagging once a real path is in', () => {
    const d = powerDownDraft();
    expect(hasUnfilledScript(d.command)).toBe(true);
    expect(hasUnfilledScript('workspacer fleet quiescence --quiet && /home/me/bin/sleep.sh')).toBe(
      false,
    );
    expect(hasUnfilledScript(undefined)).toBe(false);
  });
});
