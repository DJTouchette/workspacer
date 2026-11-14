# Sign Windows executables with Azure Trusted Signing.
#
# Run after azure/login (the module authenticates with that session). This is
# what electron-builder's azureSignOptions did for the Electron installer
# through v0.170.0, with the same account, endpoint and defaults
# (app-builder-lib windowsSignAzureManager: TrustedSigning module, SHA256
# digests, Microsoft's RFC 3161 timestamp service).
#
#   CODESIGN_PROFILE    certificate profile name (repo variable AZURE_CODESIGN_PROFILE)
#   CODESIGN_PUBLISHER  optional: the signer's subject must contain it
param([Parameter(Mandatory)][string[]]$Files)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (!$env:CODESIGN_PROFILE) { throw 'CODESIGN_PROFILE is required' }
$paths = @($Files | ForEach-Object { (Resolve-Path $_).Path })
try {
    Install-PackageProvider -Name NuGet -MinimumVersion 2.8.5.201 -Force -Scope CurrentUser | Out-Null
} catch {
    # GitHub's runners already have NuGet; Install-Module reports a real problem.
    Write-Host "NuGet provider not installed: $($_.Exception.Message)"
}
Install-Module -Name TrustedSigning -MinimumVersion 0.5.0 -Force -Repository PSGallery -Scope CurrentUser
foreach ($path in $paths) {
    Invoke-TrustedSigning `
        -Endpoint 'https://eus.codesigning.azure.net/' `
        -CodeSigningAccountName 'WorkSpacer' `
        -CertificateProfileName $env:CODESIGN_PROFILE `
        -TimestampRfc3161 'http://timestamp.acs.microsoft.com' `
        -TimestampDigest 'SHA256' `
        -FileDigest 'SHA256' `
        -Files $path
    # Never ship a file the service did not actually sign.
    $signature = Get-AuthenticodeSignature -LiteralPath $path
    if ($signature.Status -ne 'Valid') { throw "Signature is $($signature.Status): $path" }
    $subject = $signature.SignerCertificate.Subject
    if ($env:CODESIGN_PUBLISHER -and !$subject.Contains($env:CODESIGN_PUBLISHER)) {
        throw "Signed by an unexpected publisher ($subject): $path"
    }
    Write-Host "Signed by ${subject}: $path"
}
