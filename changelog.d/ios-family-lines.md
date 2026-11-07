---
section: fixed
---
- **iOS's own fonts take their own line heights** (zgx): Menlo, Courier, Georgia, Times New Roman, Helvetica and Arial take UIFont's `lineHeight` rather than 1.4 times the size, which set Menlo a fifth too tall. A font an app brings keeps 1.4 for now.
