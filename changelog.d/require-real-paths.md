---
section: performance
---
- **A script load finds where each of its folders really is once** (zgx-script): `require` followed a module's whole path at each call, to see that it isn't through a link out of the script's folder. While a host loads, a folder's real path is kept for the files in it, as long as the folder there is the same one (its device and inode), and each file is still looked at: what is refused is as it was. On Linux and macOS.
