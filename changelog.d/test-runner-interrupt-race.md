---
section: fixed
---
- **`zgx test` interrupted while a test writes leaves no directory behind** (zgx-script): on Ctrl-C (or Ctrl-Break) its cleanup removed what the run's tests made while a test went on running, so a file a test wrote in that moment could make its directory again, or keep it from being removed. The cleanup now holds the run's list from its start until the process ends, and a test makes and writes its files under that list, so a write under way finishes first and none starts after.
