param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][ValidateSet('0.1.1','0.1.2')][string]$Version,
    [ValidatePattern('^[0-9a-fA-F]{64}$')][string]$SignedSha256,
    [ValidatePattern('^[0-9a-fA-F]{40}$')][string]$PublisherThumbprint
)

$ErrorActionPreference = 'Stop'
if ([bool]$SignedSha256 -ne [bool]$PublisherThumbprint) {
    throw 'Signed verification requires the signed SHA-256 and the approved certificate thumbprint.'
}
$pinned = @{
    '0.1.1' = '164de9cfbb458f80c135fd86aac92c2da60258ad7db7e8ba06d285b72d8f2be3'
    '0.1.2' = '5f262853cc5073204fb3bdc4ff1eee0722ff0272c53e48c92a160fac6ba5ef92'
}
$expected = if ($SignedSha256) { $SignedSha256 } else { $pinned[$Version] }
$file = Get-Item -LiteralPath $Path
if ($file.PSIsContainer -or ($file.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
    throw 'Expected a regular installer file.'
}
$hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
if ($hash -ne $expected) { throw "Installer SHA-256 mismatch for $Version." }
$signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
if ($SignedSha256) {
    if ($signature.Status -ne 'Valid' -or
        $signature.SignerCertificate.Thumbprint -ne $PublisherThumbprint -or
        -not $signature.TimeStamperCertificate) {
        throw 'Expected the approved publisher and a trusted Authenticode signature with timestamp.'
    }
} elseif ($signature.Status -ne 'NotSigned') {
    throw 'The pinned QA artifact has unexpected signature status.'
}
$info = $file.VersionInfo
if ($info.ProductName -ne 'SkillVerse AI' -or $info.ProductVersion -ne $Version) {
    throw 'Installer product metadata does not match the pinned version.'
}
[ordered]@{
    file = $file.Name
    bytes = $file.Length
    sha256 = $hash.ToLowerInvariant()
    product = $info.ProductName
    version = $info.ProductVersion
    authenticode = $signature.Status.ToString()
    publisher = $signature.SignerCertificate.Subject
    timestamped = [bool]$signature.TimeStamperCertificate
}
