# D-a-session-title-costs-nothing — a title is free, or it is not a title

**Question:** a list of 500 rows needs a name per row. Where does the name come from?

## The decision

- A new session takes an automatic title from the first prompt. One line, 60 characters.
- `rho sessions name <id> "<text>"` and `/name` write a `Name` leaf record.
- The newest `Name` record wins.

No model call. A title costs no money and no time.

## Rules that hold

- `Name` is a leaf record. It is never a parent, so `D-chain-records-are-frozen` applies.
- A title is redacted on the way in, like every other record content.
- An empty title is refused, so a row never shows a blank name.
- A title is not an id. A resume never takes a title.

## Rules out

**A model-written title.** It costs a request per session, and a list must stay free.
A later feature may add one behind a config key that defaults to off.

**A title in the header.** The header is written before the first prompt arrives, and the
file is append-only, so the header can never be updated.

**A title in the file name.** A rename would change the id, and the id is the file stem.

## Cost

One leaf record, one automatic first line, and one command.
