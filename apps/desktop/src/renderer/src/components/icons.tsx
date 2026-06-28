// Centralized icon set — thin-stroke lucide glyphs replacing the old emoji /
// Unicode icons. One source of truth so every surface stays consistent.
import React from 'react';
import {
  SquareTerminal,
  FileCode2,
  Globe,
  NotebookPen,
  Bot,
  Sparkles,
  Settings,
  Search,
  Puzzle,
  Blocks,
  LayoutGrid,
  Zap,
  BarChart3,
  Brain,
  Play,
  Plus,
  FolderOpen,
  Home,
  X,
  Columns3,
  Star,
  AlertTriangle,
  RefreshCw,
  Smartphone,
  Clock,
  Rows3,
  type LucideIcon,
} from 'lucide-react';
import { PaneType } from '../types/pane';
import {
  IconTerminal,
  IconSettings,
  IconOverview,
  IconDiff,
  IconPlugin,
  IconUsage,
  IconFile,
  IconAgent,
} from './wksIcons';

// Both lucide glyphs and the Workspacer pack components accept this prop shape,
// so a pane can be backed by either set. A bare call signature (rather than
// React.ComponentType) avoids lucide's ForwardRef `propTypes` variance clash.
type IconComponent = (props: {
  size?: number | string;
  strokeWidth?: number;
  className?: string;
  style?: React.CSSProperties;
  color?: string;
}) => React.ReactNode;

const PANE_ICONS: Record<PaneType, IconComponent> = {
  // Pack glyphs where there's a clean equivalent; lucide for the rest
  // (browser/notes/library/ask/plugins have no pack counterpart).
  terminal: IconTerminal,
  browser: Globe,
  notes: NotebookPen,
  claude: IconAgent,
  settings: IconSettings,
  review: IconDiff,
  plugin: IconPlugin,
  plugins: Blocks,
  overview: IconOverview,
  library: Zap,
  analytics: IconUsage,
  ask: Brain,
  editor: IconFile,
};

export interface IconProps {
  size?: number;
  strokeWidth?: number;
  className?: string;
  style?: React.CSSProperties;
  color?: string;
}

/** Icon for a pane type. Inherits `currentColor` so it tints with the text. */
export const PaneIcon: React.FC<{ type: PaneType } & IconProps> = ({
  type,
  size = 14,
  strokeWidth = 1.75,
  ...rest
}) => {
  const Cmp = PANE_ICONS[type] ?? Globe;
  return <Cmp size={size} strokeWidth={strokeWidth} {...rest} />;
};

// Re-exports for one-off icons used outside the pane-type map.
export {
  SquareTerminal,
  Globe,
  NotebookPen,
  Bot,
  Sparkles,
  Settings,
  Search,
  Puzzle,
  Blocks,
  LayoutGrid,
  Zap,
  BarChart3,
  Brain,
  Play,
  Plus,
  FolderOpen,
  Home,
  X,
  Columns3,
  Star,
  AlertTriangle,
  RefreshCw,
  Smartphone,
  Clock,
  Rows3,
};

// Workspacer Icon Pack — the custom two-tone agent glyph set (panes, status,
// actions, diff, tools). Available app-wide as named components, a name→glyph
// registry (WKS_ICONS), and a data-driven <WksIcon name="…" /> renderer.
export * from './wksIcons';
export type { LucideIcon };
