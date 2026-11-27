# Changelog

What changed in each Warden release, newest first.

## 0.3.0

### New

- Settings has three sections, App, Appearance and Accounts, and a button beside Settings switches between light and dark.
- The Memory page shows sessions under their account: open an account to see which of its sessions used the memory. The table's checkboxes choose what the chart shows, and Warden remembers them.
- Warden's own memory has a small chart and a row of its own on the Memory page, and the Computer figure can draw the computer's memory in use on the chart.
- Processes a session left running keep counting for it after Warden restarts.

### Fixes and improvements

- The window uses less memory and less CPU: it draws on the CPU, spinners cost less to animate, and Codex usage readings that change nothing no longer reload it.
- The Overview's usage bars line up again, and an idle login's reading age moved into its bars' tooltip.
- The sidebar's theme button lights and presses like Settings.
