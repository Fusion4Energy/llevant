gaps: holes at level 0, in a filled universe and in lattice elements
1 1 -1.0 -1 imp:n=1
2 1 -1.0 -2 imp:n=1
3 0 -3 fill=1 imp:n=1
4 0 -4 fill=2 imp:n=1
8 0 -9 1 2 3 4 7 imp:n=1
9 0 9 imp:n=0
11 2 -2.0 -11 u=1 imp:n=1
12 0 11 12 u=1 imp:n=1
21 0 -22 lat=1 fill=0:1 0:1 0:0 3 3 3 3 u=2 imp:n=1
31 1 -1.0 -31 u=3 imp:n=1
32 0 31 32 u=3 imp:n=1

1 sph -20 0 0 5
2 rpp 10 20 -5 5 -5 5
3 rpp -40 -25 -10 10 -10 10
4 rpp 25 55 -15 15 -2 2
7 rpp -10 0 15 25 -5 5
9 so 100
11 sph -32 0 0 3
12 sph -32 6 0 2
22 rpp 25 35 -15 -5 -2 2
31 sph 30 -10 0 3
32 sph 32 -8 0 1

m1 1001.80c 1
m2 8016.80c 1
