$file = "D:\Development\nanna\crates\nanna-simd\src\neon.rs"
$content = Get-Content $file -Raw

# Remove unnecessary unsafe blocks around safe NEON intrinsics
$content = $content -replace 'let mut acc0 = unsafe \{ vdupq_n_f32\(0\.0\) \};', 'let mut acc0 = vdupq_n_f32(0.0);'
$content = $content -replace 'let mut acc1 = unsafe \{ vdupq_n_f32\(0\.0\) \};', 'let mut acc1 = vdupq_n_f32(0.0);'
$content = $content -replace 'let mut result = unsafe \{ vaddvq_f32\(vaddq_f32\(acc0, acc1\)\) \};', 'let mut result = vaddvq_f32(vaddq_f32(acc0, acc1));'
$content = $content -replace 'let mut dot0 = unsafe \{ vdupq_n_f32\(0\.0\) \};', 'let mut dot0 = vdupq_n_f32(0.0);'
$content = $content -replace 'let mut dot1 = unsafe \{ vdupq_n_f32\(0\.0\) \};', 'let mut dot1 = vdupq_n_f32(0.0);'
$content = $content -replace 'let mut na0 = unsafe \{ vdupq_n_f32\(0\.0\) \};', 'let mut na0 = vdupq_n_f32(0.0);'
$content = $content -replace 'let mut na1 = unsafe \{ vdupq_n_f32\(0\.0\) \};', 'let mut na1 = vdupq_n_f32(0.0);'
$content = $content -replace 'let mut nb0 = unsafe \{ vdupq_n_f32\(0\.0\) \};', 'let mut nb0 = vdupq_n_f32(0.0);'
$content = $content -replace 'let mut nb1 = unsafe \{ vdupq_n_f32\(0\.0\) \};', 'let mut nb1 = vdupq_n_f32(0.0);'
$content = $content -replace 'let mut dot_sum = unsafe \{ vaddvq_f32\(vaddq_f32\(dot0, dot1\)\) \};', 'let mut dot_sum = vaddvq_f32(vaddq_f32(dot0, dot1));'
$content = $content -replace 'let mut mag_a = unsafe \{ vaddvq_f32\(vaddq_f32\(na0, na1\)\) \};', 'let mut mag_a = vaddvq_f32(vaddq_f32(na0, na1));'
$content = $content -replace 'let mut mag_b = unsafe \{ vaddvq_f32\(vaddq_f32\(nb0, nb1\)\) \};', 'let mut mag_b = vaddvq_f32(vaddq_f32(nb0, nb1));'
$content = $content -replace 'let inv_vec = unsafe \{ vdupq_n_f32\(inv_norm\) \};', 'let inv_vec = vdupq_n_f32(inv_norm);'
$content = $content -replace 'let scalar_vec = unsafe \{ vdupq_n_f32\(scalar\) \};', 'let scalar_vec = vdupq_n_f32(scalar);'

Set-Content $file $content -NoNewline
Write-Host "Fixed neon.rs - removed unnecessary unsafe blocks"
