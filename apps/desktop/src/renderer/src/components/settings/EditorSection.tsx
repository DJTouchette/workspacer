import React from 'react';
import { Config } from '../../hooks/useConfig';
import { Section, Row, ModeButton, inputStyle } from './primitives';

interface EditorSectionProps {
  config: Config;
  save: (partial: Partial<Config>) => Promise<Config>;
}

const EditorSection: React.FC<EditorSectionProps> = ({ config, save }) => {
  const editorCfg = config.editor ?? { engine: 'codemirror' as const, terminalCommand: 'nvim' };
  const engine = editorCfg.engine;

  return (
    <Section title="Editor">
      <Row label="Open files with">
        <div style={{ display: 'flex', gap: 4 }}>
          {/* The stored value stays 'codemirror' for config compatibility, but
              that path now opens the sandboxed editor *plugin* (the in-app
              CodeMirror editor was removed) — label it for what it does. */}
          <ModeButton
            label="Editor plugin"
            active={engine !== 'terminal'}
            onClick={() => save({ editor: { ...editorCfg, engine: 'codemirror' } })}
          />
          <ModeButton
            label="Terminal"
            active={engine === 'terminal'}
            onClick={() => save({ editor: { ...editorCfg, engine: 'terminal' } })}
          />
        </div>
      </Row>
      <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
        <strong>Editor plugin</strong> opens files in the sandboxed editor plugin (or the OS default
        editor when the plugin isn't installed). <strong>Terminal</strong> runs your own editor in a
        PTY pane.
      </div>

      {engine === 'terminal' && (
        <>
          <Row label="Terminal command">
            <input
              style={{ ...inputStyle, width: 160 }}
              value={editorCfg.terminalCommand}
              spellCheck={false}
              onChange={(e) => save({ editor: { ...editorCfg, terminalCommand: e.target.value } })}
              placeholder="nvim"
            />
          </Row>
          <div style={{ fontSize: '0.72rem', color: 'var(--wks-text-disabled)' }}>
            The file path is appended as the last argument — e.g. <code>nvim</code> opens
            <code> nvim &lt;file&gt;</code>. Must be on the daemon host's PATH.
          </div>
        </>
      )}
    </Section>
  );
};

export default EditorSection;
