/**
 * The ACCOUNT allowance behind one session, on a session surface.
 *
 * This is the FIRST block in the Inspector's Usage tab. Everything after it is
 * this session's own spending: its context window, its billed tokens, its cost.
 * This block is the opposite kind of number, the provider allowance the session
 * draws from, which every other session on the same login is spending at the
 * same time. The heading and the subline say so, because a 91% weekly figure
 * sitting under "Usage" with no label reads as "this session burned 91%", which
 * is almost never true.
 *
 * It is also the ONLY place in the tab that draws account quota bars. The tab
 * used to draw them twice, once from this report and once from the opening
 * session's live status line, with pace colours on one and raw severity colours
 * on the other; two 5-hour percentages an arm's length apart, coloured by
 * different rules, is a readout that cannot be trusted at a glance.
 *
 * It is a thin wrapper on purpose. The report, its once-a-minute shared fetch,
 * the pace arithmetic and the card are all the Overview's, unchanged:
 *
 *   - `useUsageReport` — ONE cached report and one poll for the whole app, no
 *     matter how many Inspectors are open (see the hook's subscriber set). This
 *     component mounts only while the Usage tab is showing, so an Inspector on
 *     another tab subscribes to nothing.
 *   - `usageReportAttribution` — whether this session's identity actually names
 *     a row in that report. It says "ambiguous" rather than picking.
 *   - `UsageReportCard` — the same card, the same pace tick, the same
 *     above-pace colour, in its compact layout.
 *
 * What this component owns is the naming of the four honest absences, which is
 * the whole reason it is not just a card: no report at all (an older hub or a
 * failed fetch — render nothing, exactly as before the report existed), no row
 * for the provider, a session on a peer hub, and two logins where the session
 * names neither.
 */
import React from 'react';
import { claudeColors as colors } from './claude-shared';
import { useUsageReport } from '../hooks/useUsageReport';
import { usagePacingRows, usageReportAttribution } from '../lib/usagePacing';
import { UsageReportCard } from './UsageReportCard';
import { UsageSectionHeading } from './UsageSectionHeading';

/** Just the identity fields — so the block can be reasoned about (and tested)
 *  without a whole session snapshot. */
export interface SessionAccountIdentity {
  provider?: string | null;
  /** Claude's config root, and therefore its login. Blank on federated rows. */
  transcriptPath?: string;
  /** Peer hub this session lives on; absent for local sessions. */
  hub?: string;
}

const Note: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <div style={{ fontSize: '0.68rem', color: colors.muted, lineHeight: 1.45 }}>{children}</div>
);

export const SessionAccountUsage: React.FC<{ session: SessionAccountIdentity | null }> = ({
  session,
}) => {
  const report = useUsageReport();
  const attribution = usageReportAttribution(report, session);
  // One clock for the question and the answer: `usagePacingRows` drops a window
  // whose reset has passed, so asking twice a millisecond apart could leave the
  // heading standing over a card that decided it had nothing to draw.
  const nowMs = Date.now();
  // No report is not a state to describe: it is every backend that predates
  // `usage.report`, plus every minute a fetch is failing. Saying nothing leaves
  // the tab byte-for-byte what it was.
  if (attribution.state === 'unavailable' || !report) return null;
  const { provider } = attribution;
  return (
    // First block in the Usage tab, so the spacing is BELOW it: the account
    // allowance is the question a reader arrives with, and this session's own
    // spending is read against it.
    <div style={{ marginBottom: 16 }} data-testid="session-account-usage">
      <UsageSectionHeading title="Account allowance">
        The {provider} account this session spends from, shared with its other sessions, not this
        session&rsquo;s own tokens.
      </UsageSectionHeading>
      {attribution.state === 'match' ? (
        usagePacingRows(report, attribution.account, nowMs).rows.length ? (
          <UsageReportCard
            report={report}
            account={attribution.account}
            provider={provider}
            compact
            nowMs={nowMs}
          />
        ) : (
          // A row with no CURRENT window: every window it reported has rolled
          // over (its percentage is real history and a false present — see
          // usageReport.ts) or its plan reports none at all.
          <Note>No {provider} allowance window is currently running for this account.</Note>
        )
      ) : attribution.state === 'remote' ? (
        <Note>
          Not shown: this session runs on hub <strong>{attribution.hub}</strong>, and this reading
          covers accounts on this machine only.
        </Note>
      ) : attribution.state === 'ambiguous' ? (
        <Note>
          Not shown: {attribution.count} {provider} accounts are reported here and this
          session&rsquo;s account could not be identified. Overview lists them all.
        </Note>
      ) : (
        <Note>No {provider} account allowance was reported for this session.</Note>
      )}
    </div>
  );
};
