<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# File: New, Open, Save, Close

| Command | Key | What it does |
| --- | --- | --- |
| New… | Ctrl+N | Creates a part from the millimetre or inch template: a datum CSYS at the absolute origin. Choose the units in the prompt. The default comes from the `units` customer default |
| Open… | Ctrl+O | Opens a `.pxp` part. The prompt lists the parts in the folder of the path you type |
| Save | Ctrl+S | Saves atomically. If the save fails (a full disk, a lost network drive), the file on disk is untouched. The recovery journal and any autosave are cleared |
| Close | | Closes the part. Unsaved work stays in the recovery journal and is offered at the next open |

A part can be open in only one session at a time. A second session opens it
read-only and says why on the status line.

Parts written by a newer PXP version are refused with a message, never
half-read. Parts from an older version are migrated on open, and the
original is kept as a backup.
