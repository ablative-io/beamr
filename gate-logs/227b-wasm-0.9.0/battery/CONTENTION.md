# Battery contention disclosure — REQUIRED READING BEFORE TRUSTING TIMINGS

Run start 2026-08-18T02:25:44Z at pin a3b87e6.
Box measured CONTENDED immediately before launch:

    12:25  up 17:34, 3 users, load averages: 95.28 87.12 76.05
    ncpu: 10
    vm.swapusage: total = 0.00M  used = 0.00M  free = 0.00M  (encrypted)

Load ~6x oversubscribed (10 cores) — many lanes building estate-wide. Per
Waffles' execution ruling (DM 3d455b29, 2026-08-18): running contended is
sanctioned ONLY with a complete capture and the contention stated in the log —
this file is that statement. TIMINGS FROM THIS RUN ARE NOT PRICE POINTS and
must not join any battery-price bank. Verdicts (rc, axes, marker) are
load-independent and stand.
