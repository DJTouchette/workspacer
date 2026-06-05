// Centralized icon set — thin-stroke lucide glyphs replacing the old emoji /
// Unicode icons. One source of truth so every surface stays consistent.
import React from 'react';
import {
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
  type LucideIcon,
} from 'lucide-react';
import { PaneType } from '../types/pane';

const PANE_ICONS: Record<PaneType, LucideIcon> = {
  terminal: SquareTerminal,
  browser: Globe,
  notes: NotebookPen,
  agent: Bot,
  claude: Sparkles,
  settings: Settings,
  review: Search,
  plugin: Puzzle,
  plugins: Blocks,
  overview: LayoutGrid,
  library: Zap,
  analytics: BarChart3,
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
export type { LucideIcon };
