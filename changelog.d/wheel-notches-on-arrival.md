---
section: fixed
---
- **A mouse wheel's notches show on every refresh** (zgx-desktop, macOS): each notch now moves the view as it arrives and draws at once, as an `NSScrollView` takes it, and its frame hands over at the refresh grid's point. Notches used to wait for the next refresh's tick: the system hands them over about once a refresh, near its start, so one arriving just after a tick waited a whole refresh and that refresh showed nothing new. Real wheel notches at 144 Hz into a 7,000-row list on a 120 Hz display showed a new frame on 104-113 refreshes a second; now 118-120, at the same latency (p50 9.2 ms from receipt to shown; p90 9.3, from 15).
