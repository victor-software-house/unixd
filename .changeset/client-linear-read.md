---
unixd: patch
---

The client searches only newly read bytes for the end of a reply, so a reply near the frame cap no longer takes seconds to read.
