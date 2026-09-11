---
description: Cross-build Switchblade for Windows and launch it on the Windows host
argument-hint: "[audio-file] [--debug] [--wait] [-n]"
allowed-tools: Bash(scripts/win-run.sh:*)
---

Run `scripts/win-run.sh $ARGUMENTS` from the repo root and report the result.

The script cross-builds `x86_64-pc-windows-gnu`, stages the exe under
`%USERPROFILE%\Switchblade`, and launches it on the Windows host via PowerShell.
If it fails, read the script's error line — it names the missing piece (target,
linker, interop) rather than a generic build failure.
