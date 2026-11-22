---
status: draft
---
# pot-container: the policy-1 fixtures

What `prepare` says to each of [policy.json/1's fixtures](../../../contracts/fixtures/policy-1/). They were written
for bubblewrap and Seatbelt, so a backend built apart refuses them (rule 1); an invalid one is refused for the rule
it breaks first. Each run directory in `refusals.in/` links its `policy.json` to the fixture itself.

What each valid fixture would need from this backend, were it written for `container`, is a unit test
(`render::tests::the_policy_1_fixtures_refuse_what_this_slice_lacks`).

```console
$ pot-container --config config.json prepare --print work-bindings
? 1
pot (work): policy.json is for the bubblewrap backend, not container

$ pot-container --config config.json prepare --print seatbelt
? 1
pot (dispatch): policy.json is for the seatbelt backend, not container

```

## Invalid fixtures

```console
$ pot-container --config config.json prepare --print invalid-version-2
? 1
pot (work): policy.json version 2: this backend reads version 1 only

$ pot-container --config config.json prepare --print invalid-unknown-field
? 1
pot (work): policy.json/1: unknown field `owner`, expected one of `version`, `run`, `context`, `mode`, `access`, `backend`, `workspace`, `items`, `mounts`, `bindings`, `children`, `grants`

$ pot-container --config config.json prepare --print invalid-credential-value
? 1
pot (work): policy.json/1: unknown field `value`, expected one of `capability`, `selector`, `kit`, `request`, `socket`, `see`, `allow`, `remotes`, `key`, `via`, `notes`, `env`, `from`, `holds`, `refresh`, `server`, `url`, `header`, `scheme`, `config`

$ pot-container --config config.json prepare --print invalid-effective-unsandboxed
? 1
pot (work): policy.json/1: item unsandboxed "": an unsandboxed item in effect, which a backend never runs

$ pot-container --config config.json prepare --print invalid-capability-without-binding
? 1
pot (work): policy.json/1: item capability "status.relay": no binding

$ pot-container --config config.json prepare --print invalid-clone-in-place
? 1
pot (work): policy.json/1: workspace /tmp/pot-test/sbc: a clone in mode in-place

$ pot-container --config config.json prepare --print invalid-hidden-not-effective
? 1
pot (work): policy.json/1: item hidden "~/.ssh": a limits item that is not in effect

$ pot-container --config config.json prepare --print invalid-network-without-port
? 1
pot (work): policy.json/1: item network "example.com": not host:port

$ pot-container --config config.json prepare --print invalid-not-effective-without-why
? 1
pot (work): policy.json/1: item network "api.anthropic.com:443": not in effect, and no why

$ pot-container --config config.json prepare --print invalid-row-admitted-and-refused
? 1
pot (work): policy.json/1: children: row root:relay:relaytest is neither admitted nor refused, or both

```

## run and after

Not in this slice: they refuse. `cleanup` removes what `prepare` placed, and succeeds on a run directory it never
prepared.

```console
$ pot-container run work-bindings
? 1
pot (work): the container backend's run is not in this slice

$ pot-container after work-bindings
? 1
pot (work): the container backend's after is not in this slice

$ pot-container cleanup no-such-run

```

## Sources

- [contracts/fixtures/policy-1/](../../../contracts/fixtures/policy-1/) and its `invalid/`
- [backends/container/src/policy.rs](../../../backends/container/src/policy.rs): `parse`, `check_backend`
