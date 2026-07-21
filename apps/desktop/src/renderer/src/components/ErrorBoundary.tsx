import React from 'react';
import { AlertTriangle, RefreshCw } from './icons';

/**
 * Catches render/lifecycle errors in a subtree and shows a recoverable
 * fallback instead of letting a single component crash white-screen the whole
 * app. Wrap each independent region (sidebar, nav, each pane) so a failure
 * stays contained.
 *
 * `resetKeys` lets the boundary auto-recover: when any key changes (e.g. the
 * active pane id), the caught error clears and the children re-mount. The
 * "Try again" button does the same on demand.
 */
interface Props {
  children: React.ReactNode;
  /** Human label for the region, shown in the fallback ("Editor failed to render"). */
  label?: string;
  /** Render the compact inline fallback (for panes) vs. the centered one. */
  variant?: 'pane' | 'region';
  /** Changing any value here clears the error and re-mounts children. */
  resetKeys?: ReadonlyArray<unknown>;
  /** Custom fallback renderer; overrides the default. */
  fallback?: (error: Error, reset: () => void) => React.ReactNode;
}

interface State {
  error: Error | null;
}

export class ErrorBoundary extends React.Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    // Surface in devtools/console; the app keeps running.
    console.error(
      `[ErrorBoundary${this.props.label ? ` · ${this.props.label}` : ''}]`,
      error,
      info.componentStack,
    );
  }

  componentDidUpdate(prev: Props) {
    if (!this.state.error) return;
    const a = prev.resetKeys ?? [];
    const b = this.props.resetKeys ?? [];
    if (a.length !== b.length || a.some((v, i) => !Object.is(v, b[i]))) {
      this.reset();
    }
  }

  reset = () => this.setState({ error: null });

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    if (this.props.fallback) return this.props.fallback(error, this.reset);

    const compact = this.props.variant !== 'region';
    return (
      <div
        role="alert"
        style={{
          height: '100%',
          minHeight: compact ? 0 : 120,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 10,
          padding: 24,
          textAlign: 'center',
          color: 'var(--wks-text-secondary)',
          fontSize: '0.8rem',
        }}
      >
        <AlertTriangle size={22} style={{ color: 'var(--wks-warning)' }} />
        <div style={{ fontWeight: 600, color: 'var(--wks-text-primary)' }}>
          {this.props.label ? `${this.props.label} hit an error` : 'Something went wrong'}
        </div>
        <div
          style={{
            maxWidth: 360,
            lineHeight: 1.5,
            color: 'var(--wks-text-muted)',
            fontFamily: 'var(--claude-mono-font, monospace)',
            fontSize: '0.7rem',
            wordBreak: 'break-word',
          }}
        >
          {error.message || String(error)}
        </div>
        <button
          onClick={this.reset}
          style={{
            marginTop: 4,
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            fontSize: '0.75rem',
            fontFamily: 'inherit',
            fontWeight: 600,
            cursor: 'pointer',
            background: 'var(--wks-bg-surface)',
            color: 'var(--wks-text-primary)',
            border: '1px solid var(--wks-border-input)',
            borderRadius: 'var(--wks-radius-md)',
            padding: '6px 12px',
          }}
        >
          <RefreshCw size={13} /> Try again
        </button>
      </div>
    );
  }
}

export default ErrorBoundary;
