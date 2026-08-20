# D-staged-server-serves-every-connection — A test double must answer every connection

**Question (controller, id migration):** the provider contract suite failed about three runs
in eight, at the point where the stream starts, after 33 seconds. Which side is wrong, the
provider or the test double?

**Decision:** The test double. `StagedHttpServer` in `rho-provider-testkit` now accepts
every connection, serves each one in its own task, and reads the whole request before it
answers. The staged delay in the two provider harnesses drops from 30 seconds to 5, which
stays far above the 2-second window the contract asserts.

**Reason:** The first version accepted one connection, served it, and ended its task. A
provider that retries then found a listening socket with nobody to accept, so the connection
sat in the kernel backlog until the client timed out. That is why the failure took 33
seconds and why it looked like a provider fault. The 30-second delay made every failure slow
to see.

It also read the request with one `read`. One read returns one segment. A client that writes
its head and its body separately could still be writing when the server answered and closed,
and it then saw a reset instead of a response. So the double now reads until the head
terminator, then reads the body that `Content-Length` names.

**Proof.** With the one-shot accept restored on purpose, 4 of 12 runs failed. With the fix,
0 of 10 runs failed. The file was restored by copy, never by `git checkout`. See
`D-jcode-bash-lessons`.

**Why this matters more than one flaky test.** `rho-provider-testkit` is the crate a third
party uses to prove its own provider conforms. A harness that fails three runs in eight
teaches that author to distrust the suite, and a distrusted suite gets skipped. A test
double that cannot answer twice lies about the client it measures.

**Rules out:** A one-shot test server. A staged delay longer than the client timeout, which
converts a failure into a wait. A single `read` as a stand-in for reading a request.
