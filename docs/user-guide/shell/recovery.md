<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# Recovery, autosave and crash reports

- **Recovery journal**: every committed action is written to
  `part.pxp.journal` before the command returns. If PXP stops before you
  save (a crash, a power cut, a killed process), the next open offers
  **Restore unsaved work**, which brings back everything up to the last
  committed action.
- **Autosave**: every 5 minutes (the `autosave_minutes` customer default),
  a modified part is copied to `part.pxp.autosave`. This copy is used only
  if the journal is lost or damaged. Saving removes it.
- **Crash reports**: if PXP panics, a text report is written to your own
  computer. Nothing is sent anywhere. **Help** shows the folder, and
  `pxp-cli crashes` lists the reports (`--clear` deletes them). Reports
  contain the error, where it happened and the program version, but no
  part contents. Read a report before you share it.
