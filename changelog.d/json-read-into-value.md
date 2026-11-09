---
section: performance
---
- **JSON is read straight into script values** (zgx-script): `ui.json.decode`, `Value::from_json` and JSON replies no longer build serde_json's tree and then a copy of it, and each object key is one string however many objects name it. A 6.8 MB feed of 10,000 messages reads in 282M instructions where it took 685M, and drops in 92M where it took 150M. `Value::from_json` no longer parses its text twice.
