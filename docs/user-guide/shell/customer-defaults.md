<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# Customer Defaults

Settings come in three levels, and a higher level wins:

1. **Shipped**: built into PXP.
2. **Site**: the file named by the `PXP_SITE_DEFAULTS` environment
   variable, kept by your CAD administrator. Writing `key != value` there
   locks the key, so users cannot change it.
3. **User**: your own file. **Customer Defaults** (ribbon, top right)
   edits and saves it: `%APPDATA%\pxp\user_defaults.txt` on Windows,
   `~/Library/Preferences/pxp/user_defaults.txt` on macOS, and
   `~/.config/pxp/user_defaults.txt` on Linux.

The file format is `key = value`, one per line, with `#` for comments.

| Key | Values | Meaning |
| --- | --- | --- |
| `units` | `mm`, `in` | Units offered for new parts |
| `theme` | `light`, `dark` | Colour theme |
| `role` | `essentials`, `advanced` | Ribbon tabs shown |
| `autosave_minutes` | number, `0` = off | Autosave interval for a modified part |
| `edges`, `silhouettes` | `true`, `false` | Graphics display |
| `projection` | `orthographic`, `perspective` | Starting projection |
| `shortcut.<command id>` | a chord such as `Ctrl+B`, empty to remove | Remaps a command's shortcut |

Command ids are listed by `pxp-cli commands`. If two commands claim one
shortcut, the remapped one keeps it, and the conflict is reported under
**Help**.
