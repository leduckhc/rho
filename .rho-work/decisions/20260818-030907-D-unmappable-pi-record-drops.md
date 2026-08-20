# D-unmappable-pi-record-drops — An unmappable pi record drops with a count, and the import finishes

**Question (controller, T5b verification):** what does the pi importer do with a record
type, a role, or a content block that rho cannot map?

**Decision:** It drops the record and counts the drop. `import_pi_session` returns a
`PiImport` with the entries and a `dropped` map, keyed by the pi type or by the reason. The
import does not stop. A malformed line is still a hard `SessionError::Decode`, because a
line that is not JSON means the file is not a pi session file.

**Reason:** The first implementation returned an error for an unknown record type, an
unknown role, and an `image` block. Three fixture tests passed. Then the controller ran the
importer over 60 real pi session files, and **21 of them failed**. The causes were
`custom_message` (45 records), `compaction` (5), the `bashExecution` role, and the `image`
block (170). A fixture describes what an author expects. A real file holds what exists. So
the rule follows the real file: an auxiliary record must not fail a whole import. After the
fix, 60 of 60 files import, and 15646 records map.

**Rules out:** A silent drop. Every drop is counted, and the count names the type or the
role, so a caller can see what it lost. Also ruled out: an import that stops on one
unknown record, and a fixture-only proof for a format that another program writes.
