# Windows code signing (Azure Trusted Signing)

Windows release builds are signed via **Azure Trusted Signing** (formerly Azure
Code Signing) using **GitHub OIDC** — no signing secret is stored in GitHub.
Signing runs only on **tag builds** (`v*`) once the cert profile is configured;
PRs and the macOS/Linux legs are unaffected.

## Already provisioned (via `az`, 2026-06-30)

- Trusted Signing account **`WorkSpacer`** — RG `workspacer`, region `eastus`,
  SKU **Basic**, endpoint `https://eus.codesigning.azure.net/`.
- App registration / service principal **`workspacer-trusted-signing-ci`**
  - client (app) id: `e4b9c937-ef14-47cc-a43f-2a5ba3437dbc`
  - tenant id: `071580bb-b38f-44d0-97e0-76efbe7755a6`
  - subscription id: `a6cc484d-d6fb-4d6a-99b2-5c7090355522`
- Role **`Artifact Signing Certificate Profile Signer`** granted to the SP,
  scoped to the `WorkSpacer` account.
- OIDC **federated credential** on the SP for subject
  `repo:DJTouchette/workspacer:environment:release`.

## Remaining manual steps (do these before tagging a signed release)

1. **Identity validation** (Azure portal → Trusted Signing → Identity
   validations). Required for a *Public Trust* certificate profile (the kind
   Windows/SmartScreen trusts). This is a manual review and can take days. You
   need the **`Artifact Signing Identity Verifier`** role to start it.
2. **Create a certificate profile** of type **Public Trust** once validation is
   approved. Note its **name** and the **publisher common name** it issues under.
3. **GitHub → repo Settings:**
   - Create an **Environment** named `release` (no required reviewers, unless you
     want a manual gate before a signed build runs).
   - **Secrets** (Settings → Secrets and variables → Actions → Secrets):
     - `AZURE_CLIENT_ID` = `e4b9c937-ef14-47cc-a43f-2a5ba3437dbc`
     - `AZURE_TENANT_ID` = `071580bb-b38f-44d0-97e0-76efbe7755a6`
     - `AZURE_SUBSCRIPTION_ID` = `a6cc484d-d6fb-4d6a-99b2-5c7090355522`
   - **Variables** (… → Variables):
     - `AZURE_CODESIGN_PROFILE` = your certificate profile name
     - `AZURE_CODESIGN_PUBLISHER` = the publisher CN from the profile

When `AZURE_CODESIGN_PROFILE` is unset, Windows tag builds simply produce
**unsigned** installers (no failure) — so it's safe to merge this before the
Azure side is finished.

## How CI signs

`.github/workflows/release.yml`, Windows leg on a `v*` tag with the profile var set:
1. `azure/login@v2` (OIDC, `id-token: write`, job runs in the `release` environment).
2. electron-builder runs with `-c.win.azureSignOptions.*` overrides (endpoint +
   account name baked in, profile + publisher from the repo variables).
   electron-builder downloads the Trusted Signing dlib and authenticates via the
   logged-in Azure session, signing the app exe + the NSIS/portable installers.

`electron-builder.yml` is intentionally left without `azureSignOptions` so local
and PR builds stay unsigned; signing is injected only by the release workflow.

## Rotating / revoking

The SP has no secret (OIDC only). To revoke CI signing, remove the federated
credential or the role assignment:

```
az ad app federated-credential delete --id <appId> --federated-credential-id github-release-env
az role assignment delete --assignee <appId> --scope <account-resource-id> \
  --role "Artifact Signing Certificate Profile Signer"
```
