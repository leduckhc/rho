# D-one-timestamp-format — One session file holds one timestamp format, and it is epoch milliseconds

**Question (controller, T5 review):** the writer wrote epoch milliseconds, the spec
documented RFC 3339, and the pi importer carried pi's RFC 3339 strings across unchanged.
Which format wins?

**Decision:** Epoch milliseconds, as a decimal string, everywhere. rho adds no date
dependency. The pi importer converts a pi RFC 3339 timestamp into epoch milliseconds, so an
imported file matches a native file. A test asserts that every timestamp in a file parses
as a `u64`, over every record kind.

**Reason:** A resumed import would have held two formats in one file, and a reader would
have to guess per line. That is worse than either format alone. A date crate for one field
is not worth the dependency tree, and hand-written civil-date maths is a defect farm. Epoch
milliseconds need neither.

**Rules out:** A date dependency in `rho-core` for this field. A mixed-format file. A
human-readable timestamp on disk, which is the cost of this choice, paid on purpose.
