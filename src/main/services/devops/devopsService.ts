/**
 * DevOpsService — manages git/CI accounts and routes to providers.
 * Shares credential storage with TrackerService pattern.
 */
import { safeStorage } from 'electron';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import * as crypto from 'crypto';
import type {
  DevOpsProvider,
  DevOpsAccount,
  DevOpsRepo,
  DevOpsPullRequest,
  DevOpsPipeline,
  DevOpsConfigField,
  DevOpsTokenField,
} from './types';
import { AzureDevOpsGitProvider, azureDevOpsTokenField } from './azureDevOps';

const configDir = path.join(os.homedir(), '.config', 'workspacer');
const accountsFile = path.join(configDir, 'devops-accounts.json');
const tokensFile = path.join(configDir, 'devops-tokens.json');

class DevOpsService {
  private providers = new Map<string, DevOpsProvider>();
  private accounts: DevOpsAccount[] = [];
  private encryptedTokens = new Map<string, string>();

  constructor() {
    this.registerProvider(new AzureDevOpsGitProvider());
    this.load();
  }

  registerProvider(provider: DevOpsProvider): void {
    this.providers.set(provider.id, provider);
  }

  getProviderList(): Array<{ id: string; name: string; configFields: DevOpsConfigField[]; tokenField: DevOpsTokenField }> {
    return Array.from(this.providers.values()).map(p => ({
      id: p.id,
      name: p.name,
      configFields: p.configFields,
      tokenField: this.getTokenField(p.id),
    }));
  }

  private getTokenField(providerId: string): DevOpsTokenField {
    if (providerId === 'azure-devops') return azureDevOpsTokenField;
    return { label: 'Token', placeholder: 'Paste token' };
  }

  private getProvider(providerId: string): DevOpsProvider {
    const p = this.providers.get(providerId);
    if (!p) throw new Error(`Unknown DevOps provider: ${providerId}`);
    return p;
  }

  // ── Account CRUD ──

  getAccounts(): DevOpsAccount[] { return [...this.accounts]; }

  getAccount(id: string): DevOpsAccount | undefined {
    return this.accounts.find(a => a.id === id);
  }

  async addAccount(provider: string, label: string, config: Record<string, string>, token: string): Promise<DevOpsAccount> {
    const p = this.getProvider(provider);
    const account: DevOpsAccount = { id: crypto.randomUUID(), provider, label, config };
    const valid = await p.validateCredentials(account, token);
    if (!valid) throw new Error('Invalid credentials.');
    this.accounts.push(account);
    this.storeToken(account.id, token);
    this.save();
    return account;
  }

  removeAccount(accountId: string): void {
    this.accounts = this.accounts.filter(a => a.id !== accountId);
    this.encryptedTokens.delete(accountId);
    this.save();
  }

  // ── Provider passthrough ──

  async listRepos(accountId: string): Promise<DevOpsRepo[]> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.listRepos(account, token);
  }

  async listPullRequests(accountId: string, options?: { status?: 'open' | 'all'; repoId?: string }): Promise<DevOpsPullRequest[]> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.listPullRequests(account, token, options);
  }

  async listPipelines(accountId: string, options?: { maxResults?: number }): Promise<DevOpsPipeline[]> {
    const { provider, account, token } = this.resolve(accountId);
    return provider.listPipelines(account, token, options);
  }

  // ── Internal ──

  private resolve(accountId: string) {
    const account = this.accounts.find(a => a.id === accountId);
    if (!account) throw new Error(`Account not found: ${accountId}`);
    const provider = this.getProvider(account.provider);
    const token = this.retrieveToken(account.id);
    if (!token) throw new Error(`No token for account: ${account.label}`);
    return { provider, account, token };
  }

  private storeToken(accountId: string, token: string): void {
    if (safeStorage.isEncryptionAvailable()) {
      this.encryptedTokens.set(accountId, safeStorage.encryptString(token).toString('base64'));
    } else {
      this.encryptedTokens.set(accountId, Buffer.from(token).toString('base64'));
    }
  }

  private retrieveToken(accountId: string): string | null {
    const stored = this.encryptedTokens.get(accountId);
    if (!stored) return null;
    if (safeStorage.isEncryptionAvailable()) {
      try { return safeStorage.decryptString(Buffer.from(stored, 'base64')); } catch { return null; }
    }
    return Buffer.from(stored, 'base64').toString('utf-8');
  }

  private load(): void {
    try {
      if (fs.existsSync(accountsFile)) {
        const data = JSON.parse(fs.readFileSync(accountsFile, 'utf-8'));
        this.accounts = data.accounts ?? [];
      }
    } catch {}
    try {
      if (fs.existsSync(tokensFile)) {
        const data = JSON.parse(fs.readFileSync(tokensFile, 'utf-8'));
        for (const [id, enc] of Object.entries(data.tokens ?? {})) {
          this.encryptedTokens.set(id, enc as string);
        }
      }
    } catch {}
  }

  private save(): void {
    if (!fs.existsSync(configDir)) fs.mkdirSync(configDir, { recursive: true });
    fs.writeFileSync(accountsFile, JSON.stringify({ accounts: this.accounts }, null, 2));
    fs.writeFileSync(tokensFile, JSON.stringify({ tokens: Object.fromEntries(this.encryptedTokens) }, null, 2));
  }
}

export const devopsService = new DevOpsService();
