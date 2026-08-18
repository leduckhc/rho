# D-website-direction — Website direction is mockup C, with A's command-prompt labels


**Question (S10a designer):** which of the three mockups wins? The designer
picked A, the all-monospace terminal-native one.

**Decision:** Mockup **C** (`mockup-c-parts.html`) wins. Take two things from A.

**Reason:** C reads better and sells the idea faster.

- C's crate picklist, with `[x]` and `[ ]` boxes, states "you pick the parts" in
  one glance. That is the single most important message on the page. A only
  lists the crates.
- C mixes a proportional face for headings with mono for code. That gives a real
  type hierarchy. A sets body text in mono at full width, which tires the reader.
- A's `$ cat crates.txt` framing is clever, but it is a costume. The content is
  not a terminal session, so the frame fights the content.

**Take from A:** the `$ <command>` form for the small eyebrow label above each
section. It is terminal-native without dressing the whole page as a shell.

**Must fix in C:** the `[x]` and `[ ]` column is decoration to a screen reader.
Give it a real accessible label, or make it a real form control, or mark it
`aria-hidden` and put the meaning in text.
