# Battery contention disclosure — release 0.19.0 cut

Run start 2026-08-18T04:27:17Z at pin c991622.
Box state immediately before launch:

    14:27  up 19:35, 4 users, load averages: 37.64 64.19 55.51
    ncpu: 10
    vm.swapusage: total = 0.00M  used = 0.00M  free = 0.00M  (encrypted)

Load remains oversubscribed on 10 cores (estate lanes building). Sanctioned
per Waffles' contended-battery terms (DM 3d455b29): complete capture,
contention stated in-log. TIMINGS ARE NOT PRICE POINTS; verdicts (rc, marker,
axes) stand. Swap 0 used at launch.

Axes prior (from B-144 R1 battery at 2bddceb, restated at a3b87e6):
leg4 2/86/0/0 · leg5 76/2150/0/0 · leg8 76/2160/0/0. Expected delta at this
pin: NONE — the trio deletion removed no tests, the #104 demotion keeps every
scaffold test compiled via test-support, and version bumps carry no test
delta. Any axis movement is a finding, not noise.
