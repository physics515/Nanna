# Code Signing Setup for Nanna

This document describes how to set up code signing for Nanna Windows releases (P0.3 roadmap item).

## Current Status (v0.3.8-beta.13)

**Binaries are unsigned.** Users will see Windows SmartScreen warnings when running the installer
or executables. This is expected and documented in the README ("More info → Run anyway").

## Prerequisites for Windows Code Signing

### 1. Obtain a Code Signing Certificate

You need an **EV Code Signing Certificate** from a trusted Certificate Authority:

- **DigiCert** — most common for commercial software
- **Sectigo** (formerly Comodo) — lower cost option
- **GlobalSign** — enterprise option

**Important:** EV certificates are required to avoid SmartScreen warnings for new publishers.
Standard OV certificates will still trigger SmartScreen until reputation is built (requires
thousands of installs).

**Cost:** Typically $200–$500/year for EV certificates.

### 2. Hardware Token (HSM)

EV certificates are issued on hardware tokens (USB HSM) that must be:
- Present on the signing machine
- Have the certificate driver installed
- Be accessible via PKCS#11 or Windows certificate store

**For CI signing (GitHub Actions):** Consider cloud-based HSM solutions:
- **Azure Key Vault** (with Azure SignTool)
- **DigiCert KeyLocker** (cloud HSM)
- **Sectigo Certificate Manager**

## Local Signing Setup

### Install signtool

`signtool.exe` is included with the Windows SDK. Paths on this machine:
```
C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64\signtool.exe
C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\signtool.exe
C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\signtool.exe
```

### Import Certificate to Windows Store

```powershell
# Import .pfx to CurrentUser\My store
Import-PfxCertificate -FilePath "path\to\cert.pfx" -CertStoreLocation Cert:\CurrentUser\My -Password (ConvertTo-SecureString -String "password" -AsPlainText -Force)

# Verify import
Get-ChildItem -Path Cert:\CurrentUser\My -CodeSigningCert
```

### Sign Executables

```batch
rem Add signtool to PATH (use latest SDK)
set PATH=%PATH%;C:\Program Files (x86)\Windows Kits\10\bin\10.0.26100.0\x64

rem Sign with SHA-256 and timestamp
signtool sign /sha1 <CERT_THUMBPRINT> /tr http://timestamp.digicert.com /td sha256 /fd sha256 /v path\to\file.exe

rem Or sign by subject name
signtool sign /n "Your Company Name" /tr http://timestamp.digicert.com /td sha256 /fd sha256 /v path\to\file.exe
```

**Files to sign for Nanna:**
- `nanna-gui.exe` — main GUI application
- `nanna-daemon.exe` — daemon sidecar
- `Nanna_x.y.z_x64-setup.exe` — NSIS installer
- `Nanna_x.y.z_x64_en-US.msi` — MSI installer (if generated)

## CI Signing with GitHub Actions (P0.3 Target)

### Option A: Azure Key Vault (Recommended for GitHub Actions)

1. Store the certificate in Azure Key Vault
2. Use `azure-code-signing` action or Azure SignTool
3. Configure GitHub secrets:
   - `AZURE_CLIENT_ID`
   - `AZURE_TENANT_ID`
   - `AZURE_CLIENT_SECRET`
   - `AZURE_KEY_VAULT_URL`
   - `AZURE_CERT_NAME`

```yaml
# .github/workflows/release.yml snippet
- name: Sign Windows artifacts
  uses: azure/azure-code-signing@v1
  with:
    azure-tenant-id: ${{ secrets.AZURE_TENANT_ID }}
    azure-client-id: ${{ secrets.AZURE_CLIENT_ID }}
    azure-client-secret: ${{ secrets.AZURE_CLIENT_SECRET }}
    endpoint: ${{ secrets.AZURE_KEY_VAULT_URL }}
    certificate-profile-name: ${{ secrets.AZURE_CERT_NAME }}
    files-folder: ${{ github.workspace }}/target/release/bundle/nsis
    files-folder-filter: exe
    file-digest: SHA256
    timestamp-rfc3161: http://timestamp.digicert.com
    timestamp-digest: SHA256
```

### Option B: Tauri Plugin Signing (Built-in)

Tauri supports signing via environment variables:

```bash
# For .pfx file (local development / self-hosted runners)
export TAURI_SIGNING_PRIVATE_KEY=/path/to/signing.pfx
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=password

# Or base64-encoded for GitHub secrets
export TAURI_SIGNING_PRIVATE_KEY=base64-encoded-pfx-content
```

**Note:** This is for Tauri's own signature (updater verification), not Windows Authenticode.
For SmartScreen trust, you still need Authenticode signing with signtool.

### Option C: DigiCert KeyLocker (Cloud HSM)

1. Upload certificate to DigiCert KeyLocker
2. Use `smctl` CLI tool in CI
3. Configure GitHub secrets with API credentials

## Timestamp Servers

Always use a timestamp server to ensure signatures remain valid after certificate expiration:

- `http://timestamp.digicert.com` (DigiCert)
- `http://timestamp.sectigo.com` (Sectigo)
- `http://timestamp.globalsign.com` (GlobalSign)

## Verification

```batch
rem Verify signature
signtool verify /pa /v path\to\file.exe

rem Check certificate chain
signtool verify /pa /all /v path\to\file.exe
```

## Cost Summary

| Item | Cost (Annual) | Notes |
|------|--------------|-------|
| EV Code Signing Certificate | $200–$500 | Required for SmartScreen trust |
| Hardware Token (HSM) | $0–$50 | Usually included with cert |
| Cloud HSM (Azure Key Vault) | ~$0.03/signing op | For CI pipelines |
| DigiCert KeyLocker | ~$500/year | Alternative cloud HSM |

## Action Items for P0.3

1. [ ] Purchase EV code signing certificate (DigiCert recommended)
2. [ ] Set up Azure Key Vault or DigiCert KeyLocker for CI
3. [ ] Configure GitHub repository secrets
4. [ ] Update `.github/workflows/release.yml` to sign artifacts
5. [ ] Verify signed installers with `signtool verify`
6. [ ] Update README to remove SmartScreen warning note

## References

- [Tauri Code Signing Guide](https://tauri.app/distribute/sign/windows/)
- [Microsoft Authenticode](https://learn.microsoft.com/en-us/windows/win32/seccrypto/cryptography-tools)
- [Azure Code Signing Action](https://github.com/marketplace/actions/azure-code-signing)
- [DigiCert EV Code Signing](https://www.digicert.com/signing/code-signing-certificates)
