// POSITIVE CONTROL for the `blocking-call-in-native-bif` gate. NOT compiled:
// this file is outside every crate, so cargo never sees it.
//
// The gate's normal verdict is "no findings, exit 0" — which is the same
// verdict it returns when its rule matches nothing, when its rule has been
// emptied, and (measured) when its scan path does not exist at all. A zero from
// an absent check and a zero from a passing check are the same number.
//
// So the gate scans THIS file first and REQUIRES at least one finding. If the
// checker cannot fire here, the gate fails loudly instead of reporting clean.
// Every construct the rule claims to catch appears below; adding a pattern to
// the rule without adding its specimen here makes that pattern untested.

fn blocking_specimens() {
    std::thread::sleep(std::time::Duration::from_secs(1));
    thread::sleep(std::time::Duration::from_secs(1));
    std::thread::park();
    thread::park();
}
