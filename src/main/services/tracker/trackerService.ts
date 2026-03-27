/**
 * TrackerService — manages accounts, provider registry, and secure credential storage.
 * Tokens are encrypted via Electron safeStorage and persisted alongside account config.
 */
import { safeStorage } from 'electron';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import * as crypto from 'crypto';

import type {
  TrackerProvider,
  TrackerAccount,
  TrackerProject,
  TrackerIssue,
  TrackerStatus,
  TrackerTransition,
  ListIssuesOptions,
  ConfigField,
  TokenField,
} from './types';
import { JiraProvider, jiraTokenField } from './jiraProvider';
import { AzureDevOpsProvider, azureDevOpsTokenField } from './azureDevOpsProvider';

// ── Persistence paths ──

const configDir = path.join(os.homedir(), '.config', 'workspacer');
const accountsFile = path.join(configDir, 'tracker-accounts.json');
const tokensFile = path.join(configDir, 'tracker-tokens.json');

// ── Persisted data shapes ──

interface PersistedAccounts {
  accounts: TrackerAccount[];
}

interface PersistedTokens {
  /** Map of accountId → base64-encoded encrypted token */
  tokens: Record<string, string>;
}

// ── Service ──

class TrackerService {
  private providers = new Map<string, TrackerProvider>();
  private accounts: TrackerAccount[] = [];
  private encryptedTokens = new Map<string, string>(); // accountId → base64(encrypted)

  constructor() {
    this.registerProvider(new JiraProvider());
    this.registerProvider(new AzureDevOpsProvider());
    this.load();
  }

  // ── Provider registry ──

  registerProvider(provider: TrackerProvider): void {
    this.providers.set(provider.id, provider);
  }

  getProviderList(): Array<{ id: string; name: string; configFields: ConfigField[]; tokenField: TokenField }> {
    return Array.from(this.providers.values()).map((p) => ({
      id: p.id,
      name: p.name,
      configFields: p.configFields,
      tokenField: this.getTokenField(p.id),
    }));
  }

  private getTokenField(providerId: string): TokenField {
    if (providerId === 'jira') return jiraTokenField;
    if (providerId === 'azure-devops') return azureDevOpsTokenField;
    return { label: 'API Token', placeholder: 'Paste your API token' };
  }

  private getProvider(providerId: string): TrackerProvider {
    const p = this.providers.get(providerId);
    if (!p) throw new Error(`Unknown tracker provider: ${providerId}`);
    return p;
  }

  // ── Account CRUD ──

  getAccounts(): TrackerAccount[] {
    return [...this.accounts];
  }

  getAccount(accountId: string): TrackerAccount | undefined {
    return this.accounts.find((a) => a.id === accountId);
  }

  async addAccount(
    provider: string,
    label: string,
    config: Record<string, string>,
    token: string,
  ): Promise<TrackerAccount> {
    const p = this.getProvider(provider);
    const account: TrackerAccount = {
      id: crypto.randomUUID(),
      provider,
      label,
      config,
      pinnedProjects: [],
    };

    // Validate before saving
    const valid = await p.validateCredentials(account, token);
    if (!valid) throw new Error('Invalid credentials — could not authenticate with the provider.');

    this.accounts.push(account);
    this.storeToken(account.id, token);
    this.save();
    return account;
  }

  async updateAccount(
    accountId: string,
    updates: { label?: string; config?: Record<string, string>; token?: string; pinnedProjects?: string[] },
  ): Promise<TrackerAccount> {
    const account = this.accounts.find((a) => a.id === accountId);
    if (!account) throw new Error(`Account not found: ${accountId}`);

    if (updates.label !== undefined) account.label = updates.label;
    if (updates.config !== undefined) account.config = updates.config;
    if (updates.pinnedProjects !== undefined) account.pinnedProjects = updates.pinnedProjects;

    if (updates.token) {
      const p = this.getProvider(account.provider);
      const valid = await p.validateCredentials(account, updates.token);
      if (!valid) throw new Error('Invalid credentials.');
      this.storeToken(account.id, updates.token);
    }

    this.save();
    return { ...account };
  }

  removeAccount(accountId: string): void {
    this.accounts = this.accounts.filter((a) => a.id !== accountId);
    this.encryptedTokens.delete(accountId);
    this.save();
  }

  // ── Provider passthrough ──

  async validateCredentials(accountId: string): Promise<boolean> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.validateCredentials(account, token);
  }

  async listProjects(accountId: string): Promise<TrackerProject[]> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.listProjects(account, token);
  }

  async listIssues(accountId: string, options: ListIssuesOptions): Promise<TrackerIssue[]> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.listIssues(account, token, options);
  }

  async getIssue(accountId: string, issueKey: string): Promise<TrackerIssue | null> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.getIssue(account, token, issueKey);
  }

  async searchIssues(accountId: string, query: string): Promise<TrackerIssue[]> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.searchIssues(account, token, query);
  }

  async listStatuses(accountId: string, projectKey: string): Promise<TrackerStatus[]> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.listStatuses(account, token, projectKey);
  }

  async getTransitions(accountId: string, issueKey: string): Promise<TrackerTransition[]> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.getTransitions(account, token, issueKey);
  }

  async transitionIssue(accountId: string, issueKey: string, transitionId: string): Promise<void> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.transitionIssue(account, token, issueKey, transitionId);
  }

  /** Resolve an issue key across all accounts (tries each until one returns a result) */
  async resolveIssueKey(issueKey: string): Promise<TrackerIssue | null> {
    for (const account of this.accounts) {
      try {
        const { provider, token } = this.resolveById(account);
        const issue = await provider.getIssue(account, token, issueKey);
        if (issue) return issue;
      } catch { /* try next account */ }
    }
    return null;
  }

  // ── Internal helpers ──

  private resolve(accountId: string) {
    const account = this.accounts.find((a) => a.id === accountId);
    if (!account) throw new Error(`Account not found: ${accountId}`);
    return this.resolveById(account);
  }

  private resolveById(account: TrackerAccount) {
    const provider = this.getProvider(account.provider);
    const token = this.retrieveToken(account.id);
    if (!token) throw new Error(`No token stored for account: ${account.label}`);
    return { provider, account, token };
  }

  // ── Token encryption (Electron safeStorage) ──

  private storeToken(accountId: string, token: string): void {
    if (safeStorage.isEncryptionAvailable()) {
      const encrypted = safeStorage.encryptString(token);
      this.encryptedTokens.set(accountId, encrypted.toString('base64'));
    } else {
      // Fallback: store as base64 (not truly secure, but functional)
      this.encryptedTokens.set(accountId, Buffer.from(token).toString('base64'));
    }
  }

  private retrieveToken(accountId: string): string | null {
    const stored = this.encryptedTokens.get(accountId);
    if (!stored) return null;
    if (safeStorage.isEncryptionAvailable()) {
      try {
        return safeStorage.decryptString(Buffer.from(stored, 'base64'));
      } catch {
        return null;
      }
    }
    // Fallback
    return Buffer.from(stored, 'base64').toString('utf-8');
  }

  // ── Persistence ──

  private load(): void {
    try {
      if (fs.existsSync(accountsFile)) {
        const data: PersistedAccounts = JSON.parse(fs.readFileSync(accountsFile, 'utf-8'));
        this.accounts = data.accounts ?? [];
      }
    } catch (err) {
      console.error('[TrackerService] failed to load accounts:', err);
    }

    try {
      if (fs.existsSync(tokensFile)) {
        const data: PersistedTokens = JSON.parse(fs.readFileSync(tokensFile, 'utf-8'));
        for (const [id, enc] of Object.entries(data.tokens ?? {})) {
          this.encryptedTokens.set(id, enc);
        }
      }
    } catch (err) {
      console.error('[TrackerService] failed to load tokens:', err);
    }
  }

  private save(): void {
    try {
      if (!fs.existsSync(configDir)) fs.mkdirSync(configDir, { recursive: true });

      const accountData: PersistedAccounts = { accounts: this.accounts };
      fs.writeFileSync(accountsFile, JSON.stringify(accountData, null, 2));

      const tokenData: PersistedTokens = { tokens: Object.fromEntries(this.encryptedTokens) };
      fs.writeFileSync(tokensFile, JSON.stringify(tokenData, null, 2));
    } catch (err) {
      console.error('[TrackerService] failed to save:', err);
    }
  }
}

export const trackerService = new TrackerService();
