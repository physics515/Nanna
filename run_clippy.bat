@echo off
cd /d D:\Development\nanna
cargo clippy --all-targets 2>&1 | findstr /c:"warning:" > clippy_warnings.txt
type clippy_warnings.txt
