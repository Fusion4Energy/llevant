Example for the llevant docs: a small PWR-like core (not a real design)
c Written by docs/examples/gen_core.py.
c --- universe 1: a fuel pin
1  3 -10.4   -1             u=1 imp:n=1  $ fuel
2  0          1 -2          u=1 imp:n=1  $ gap
3  2 -6.5     2 -3          u=1 imp:n=1  $ cladding
4  1 -1.0     3             u=1 imp:n=1  $ water
c --- universe 2: a guide tube
5  1 -1.0    -4             u=2 imp:n=1  $ water in the tube
6  2 -6.5     4 -5          u=2 imp:n=1  $ guide tube
7  1 -1.0     5             u=2 imp:n=1  $ water
c --- universe 3: 17 x 17 pins
8  1 -1.0  10 -11 12 -13 lat=1 u=3 imp:n=1 fill=-8:8 -8:8 0:0 1 1 1 1 1 1 1 1
     1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 2 1 1 2 1 1
     2 1 1 1 1 1 1 1 1 2 1 1 1 1 1 1 1 1 1 2 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1
     1 1 1 1 1 2 1 1 2 1 1 2 1 1 2 1 1 2 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1
     1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 2 1 1 2 1 1 2 1 1 2 1 1 2 1 1 1 1 1
     1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 2 1 1 2
     1 1 2 1 1 2 1 1 2 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 2 1 1 1 1 1
     1 1 1 1 2 1 1 1 1 1 1 1 1 2 1 1 2 1 1 2 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1
     1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1 1
c --- universe 4: an assembly; universe 5: water
9  0         20 -21 22 -23  fill=3 u=4 imp:n=1  $ assembly
10 1 -1.0   -20:21:-22:23        u=4 imp:n=1  $ water around it
11 1 -1.0   -99                  u=5 imp:n=1  $ water
12 1 -1.0    99                  u=5 imp:n=1  $ water
c --- universe 6: 17 x 17 assemblies
13 1 -1.0  20 -21 22 -23 lat=1 u=6 imp:n=1 fill=-8:8 -8:8 0:0 5 5 5 5 5 5 5 5
     5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 4 4 4 4 4 5 5 5 5 5 5 5 5 5 5 4 4 4 4 4 4 4
     4 4 5 5 5 5 5 5 5 4 4 4 4 4 4 4 4 4 4 4 5 5 5 5 5 4 4 4 4 4 4 4 4 4 4 4 4
     4 5 5 5 5 4 4 4 4 4 4 4 4 4 4 4 4 4 5 5 5 4 4 4 4 4 4 4 4 4 4 4 4 4 4 4 5
     5 4 4 4 4 4 4 4 4 4 4 4 4 4 4 4 5 5 4 4 4 4 4 4 4 4 4 4 4 4 4 4 4 5 5 4 4
     4 4 4 4 4 4 4 4 4 4 4 4 4 5 5 4 4 4 4 4 4 4 4 4 4 4 4 4 4 4 5 5 5 4 4 4 4
     4 4 4 4 4 4 4 4 4 5 5 5 5 4 4 4 4 4 4 4 4 4 4 4 4 4 5 5 5 5 5 4 4 4 4 4 4
     4 4 4 4 4 5 5 5 5 5 5 5 4 4 4 4 4 4 4 4 4 5 5 5 5 5 5 5 5 5 5 4 4 4 4 4 5
     5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5 5
c --- the reactor
20 0        -50 40 -41   fill=6     imp:n=1  $ core
21 4 -7.9   -50 42 -40              imp:n=1  $ lower core plate
22 4 -7.9   -50 41 -43              imp:n=1  $ upper core plate
23 1 -1.0   -50 52 -42              imp:n=1  $ lower plenum
24 1 -1.0   -50 43 -53              imp:n=1  $ upper plenum
25 4 -7.9    50 -51 52 -53          imp:n=1  $ core barrel
26 1 -1.0   -54 56 -57 (51:-52:53)  imp:n=1  $ downcomer
27 4 -7.9   -55 58 -59 (54:-56:57)  imp:n=1  $ vessel
28 6 -2.3    60 -61 62 -63          imp:n=1  $ concrete shield
29 6 -2.3   -61 64 -62              imp:n=1  $ concrete floor
30 5 -0.0012 -98 (55:-58:59) #28 #29 imp:n=1  $ air
31 0         98                     imp:n=0  $ outside world

1  cz 0.41
2  cz 0.418
3  cz 0.475
4  cz 0.56
5  cz 0.61
10 px -0.63
11 px 0.63
12 py -0.63
13 py 0.63
20 px -10.71
21 px 10.71
22 py -10.71
23 py 10.71
40 pz -183
41 pz 183
42 pz -200
43 pz 200
50 cz 175.0
51 cz 180
52 pz -220
53 pz 220
54 cz 195
55 cz 215
56 pz -240
57 pz 240
58 pz -260
59 pz 260
60 cz 240
61 cz 330
62 pz -300
63 pz 300
64 pz -350
98 so 600
99 so 1e9

m1 1001 2  8016 1                                  $ water
m2 40090 0.98  50120 0.015  26056 0.005            $ zircaloy
m3 92235 0.04  92238 0.96  8016 2                  $ uranium dioxide
m4 26056 0.70  24052 0.19  28058 0.11              $ stainless steel
m5 7014 0.79  8016 0.21                            $ air
m6 1001 0.17  8016 0.56  14028 0.20  20040 0.06  13027 0.01   $ concrete
nps 1
