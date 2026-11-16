Example for the llevant docs: a steel cask of fuel pins on a concrete floor
c
c Not a real design: a small input to try llevant on (docs/getting-started.md).
c A steel cask (cells 2 and 3) holds a water-filled cavity (cell 4). The cavity
c is filled with a square lattice of fuel pins (universe 1, cell 30), each pin
c being universe 2 (cells 41-43). The cask stands on a concrete floor (cell 1)
c in air (cell 5); cell 6 is the outside world.
c
c --- cells --------------------------------------------------------------------
1   5 -2.3     -1                   imp:n=1  $ concrete floor
2   4 -7.9     -10 11               imp:n=1  $ cask body
3   4 -7.9     -12                  imp:n=1  $ cask lid
4   0          -11  fill=1          imp:n=1  $ cavity, filled by the lattice
5   6 -0.0012  -20 1 10 12          imp:n=1  $ air
6   0           20                  imp:n=0  $ outside world
c --- universe 1: a square lattice of pins, 10 cm pitch
30  0          -31  lat=1 fill=2 u=1 imp:n=1  $ lattice of pins
c --- universe 2: one fuel pin in water
41  2 -10.4    -43             u=2  imp:n=1  $ fuel
42  3 -6.5     -44 43          u=2  imp:n=1  $ cladding
43  1 -1.0      44             u=2  imp:n=1  $ water

c --- surfaces -----------------------------------------------------------------
1   rpp -150 150 -150 150 -50 0          $ floor
10  rcc 0 0 0   0 0 180  90              $ cask body, outside
11  rcc 0 0 20  0 0 160  70              $ cavity
12  rcc 0 0 180 0 0 25   90              $ lid
20  rpp -200 200 -200 200 -50 300        $ world
31  rpp -5 5 -5 5 -1000 1000             $ lattice element
43  rcc 0 0 30  0 0 140  3.6             $ fuel
44  rcc 0 0 28  0 0 144  4.0             $ cladding

c --- materials ----------------------------------------------------------------
m1  1001 2  8016 1                                  $ water
m2  92235 0.04  92238 0.96  8016 2                  $ uranium dioxide
m3  40090 1                                         $ zirconium
m4  26056 0.70  24052 0.19  28058 0.11              $ stainless steel
m5  1001 0.17  8016 0.56  14028 0.20  20040 0.06  13027 0.01   $ concrete
m6  7014 0.79  8016 0.21                            $ air
nps 1
