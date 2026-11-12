---
section: fixed
---
- **A pointer press on a toast leaves focus where it was** (zgx-components): a toast's card, action and Dismiss button took keyboard focus on a press, and removing the toast then handed focus to an unrelated control, which scrolled to reveal itself, even off screen. They now take the click only, as the toast docs promise; F8, Tab and the keyboard still reach them.
