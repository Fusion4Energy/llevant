universe zoo for llevant / MCNP comparison
c --- slot 0: fill with TR reference (cosines)
10 0 -5000 fill=1 (1) imp:n=1
c --- slot 1: *fill inline degrees
11 0 -5001 *fill=1 (44.0 0 0  30 60 90  120 30 90  90 90 0) imp:n=1
c --- slot 2: fill inline displacement
12 0 -5002 fill=1 (74.0 1 2) imp:n=1
c --- slot 3: nested: fill=2 with no transform, universe 2 fills 1
13 0 -5003 fill=2 imp:n=1
c --- slot 4: fill=2 with TR m=-1
14 0 -5004 fill=2 (4) imp:n=1
c --- slot 5: lattice (RPP element), translated and rotated
15 0 -5005 *fill=4 (164.0 0.4 -0.7  20 70 90  110 20 90  90 90 0) imp:n=1
c --- slot 6: 2-D lattice from planes
16 0 -5006 fill=6 (194.0 0.25 0) imp:n=1
c --- slot 7: skewed lattice
17 0 -5007 fill=7 (224.0 0 0.3) imp:n=1
c --- slot 8: universe containing a lattice
18 0 -5008 fill=8 (254.0 0 0) imp:n=1
c --- slot 9: TRCL inline displacement
400 1 -1.0 -301 302 -303 trcl=(284.0 1 0) imp:n=1
401 2 -2.0 -5009 #400 imp:n=1
c --- slot 10: *TRCL inline rotation
402 1 -1.0 -301 302 -303 *trcl=(314.0 0 0  30 60 90  120 30 90  90 90 0) imp:n=1
403 2 -2.0 -5010 #402 imp:n=1
c --- slot 11: TRCL by TR number
404 1 -1.0 -301 302 -303 trcl=111 imp:n=1
405 2 -2.0 -5011 #404 imp:n=1
c --- slot 12: LIKE BUT TRCL (overrides the base cell's TRCL)
406 like 400 but trcl=(374.0 0.5 -1) imp:n=1
407 2 -2.0 -5012 #406 imp:n=1
c --- slot 13: TRCL on a filled cell
408 0 -304 trcl=(404.0 0 0  0.8 0.6 0  -0.6 0.8 0  0 0 1) fill=1 imp:n=1
409 2 -2.0 -5013 #408 imp:n=1
c --- slot 14: TRCL and fill transform
410 0 -304 *trcl=(434.0 0 1  0 90 90  90 0 90  90 90 0) fill=1 (2 1 0) imp:n=1
411 2 -2.0 -5014 #410 imp:n=1
c --- slot 15: fill array lattice
25 0 -5015 fill=9 (464.0 0 0) imp:n=1
c --- slot 16: 2-D hexagonal lattice
26 0 -5016 fill=10 (494.0 0.4 0) imp:n=1
c --- slot 17: 3-D hexagonal lattice (RHP element)
27 0 -5017 *fill=11 (524.0 0 0  20 70 90  110 20 90  90 90 0) imp:n=1
c --- slot 18: lattice cell with TRCL
28 0 -5018 fill=12 (554.0 0 0) imp:n=1
c --- universe 1
201 1 -1.0 -101 u=1 imp:n=1
202 2 -2.0 101 -102 u=1 imp:n=1
203 0 101 102 u=1 imp:n=1
c --- universe 5
251 1 -1.0 -151 u=5 imp:n=1
252 0 151 u=5 imp:n=1
c --- universe 2: nests universe 1
211 0 -111 fill=1 (3) u=2 imp:n=1
212 2 -2.0 111 u=2 imp:n=1
c --- universe 4: LAT=1 with an RPP element
261 0 -161 lat=1 fill=5 u=4 imp:n=1
c --- universe 6: 2-D LAT=1 from four planes
271 0 -171 172 -173 174 lat=1 fill=5 u=6 imp:n=1
c --- universe 7: skewed hexahedral lattice
281 0 -181 182 -183 184 -185 186 lat=1 fill=5 u=7 imp:n=1
c --- universe 8: a sphere filled with the RPP lattice
291 0 -191 fill=4 (0.3 0.2 0.1) u=8 imp:n=1
292 2 -2.0 191 u=8 imp:n=1
c --- universe 9: LAT=1 with a fill array (own universe, transforms)
301 3 -3.0 -311 lat=1 u=9 imp:n=1 fill=-3:3 -2:2 0:0
     5 1 9 5 (0.5 0 0) 1 5 9
     1 5 5 (4) 9 1 5 1
     9 9 1 5 1 5 (0 0.2 0) 9
     5 1 1 9 5 1 5
     1 5 9 1 5 9 1
c --- universe 10: 2-D hexagonal lattice from planes, fill array
302 0 -321 322 -323 324 -325 326 lat=2 u=10 imp:n=1 fill=-8:8 -4:4 0:0
     5 1 10 5 5 1 5 1 10 5 5 1 5 1 10 5 5
     5 5 1 5 1 10 5 5 1 5 1 10 5 5 1 5 1
     5 1 10 5 5 1 5 1 10 5 5 1 5 1 10 5 5
     5 5 1 5 1 10 5 5 1 5 1 10 5 5 1 5 1
     5 1 10 5 5 1 5 1 10 5 5 1 5 1 10 5 5
     5 5 1 5 1 10 5 5 1 5 1 10 5 5 1 5 1
     5 1 10 5 5 1 5 1 10 5 5 1 5 1 10 5 5
     5 5 1 5 1 10 5 5 1 5 1 10 5 5 1 5 1
     5 1 10 5 5 1 5 1 10 5 5 1 5 1 10 5 5
c --- universe 11: 3-D hexagonal lattice from an RHP, uniform fill
303 0 -331 lat=2 fill=5 u=11 imp:n=1
c --- universe 12: a LAT=1 cell moved by TRCL
304 0 -311 lat=1 fill=5 u=12 imp:n=1 trcl=(0.3 0.1 0  0.8 0.6 0  -0.6 0.8 0  0 0 1)
998 0 -5999 5000 5001 5002 5003 5004 5005 5006 5007 5008 5009 5010 5011 5012 5013 5014 5015 5016 5017 5018 imp:n=1
999 0 5999 imp:n=0

101 s 2 1 0 3
102 px -1
151 s 0.5 0.3 0 1.2
111 so 9
161 rpp -2 2 -3 3 -1.5 1.5
171 px 1.5
172 px -1.5
173 py 2
174 py -2
181 p 1 -0.5 0 1.5
182 p 1 -0.5 0 -1.5
183 py 2
184 py -2
185 pz 1.7
186 pz -1.7
191 so 12
301 c/z 1 0 3
302 pz -4
303 pz 5
304 so 6
311 rpp -2 2 -3 3 -15 15
321 p 1 0 0 2
322 p 1 0 0 -2
323 p 0.5 0.866025403784439 0 2
324 p 0.5 0.866025403784439 0 -2
325 p -0.5 0.866025403784439 0 2
326 p -0.5 0.866025403784439 0 -2
327 pz 3
328 pz -3
331 rhp 0 0 -2  0 0 4  0 1.8 0
341 so 11
5000 rpp 0 28 -14 14 -14 14
5001 rpp 30 58 -14 14 -14 14
5002 rpp 60 88 -14 14 -14 14
5003 rpp 90 118 -14 14 -14 14
5004 rpp 120 148 -14 14 -14 14
5005 rpp 150 178 -14 14 -14 14
5006 rpp 180 208 -14 14 -14 14
5007 rpp 210 238 -14 14 -14 14
5008 rpp 240 268 -14 14 -14 14
5009 rpp 270 298 -14 14 -14 14
5010 rpp 300 328 -14 14 -14 14
5011 rpp 330 358 -14 14 -14 14
5012 rpp 360 388 -14 14 -14 14
5013 rpp 390 418 -14 14 -14 14
5014 rpp 420 448 -14 14 -14 14
5015 rpp 450 478 -14 14 -14 14
5016 rpp 480 508 -14 14 -14 14
5017 rpp 510 538 -14 14 -14 14
5018 rpp 540 568 -14 14 -14 14
5999 rpp -10 580 -20 20 -20 20

m1 1001.32c 1
m2 26056.32c 1
m3 8016.32c 1
tr1 14.0 1 -1  0.866025403784 0.5 0  -0.5 0.866025403784 0  0 0 1
*tr3 0 2 0  90 0 90  180 90 90  90 90 0
tr4 -134.0 0 -1  1 0 0  0 0 1  0 -1 0  -1
tr109 284.0 -1 0.5  0 1 0  -1 0 0  0 0 1
tr110 314.0 -1 0.5  0 1 0  -1 0 0  0 0 1
tr111 344.0 -1 0.5  0 1 0  -1 0 0  0 0 1
tr112 374.0 -1 0.5  0 1 0  -1 0 0  0 0 1
tr113 404.0 -1 0.5  0 1 0  -1 0 0  0 0 1
tr114 434.0 -1 0.5  0 1 0  -1 0 0  0 0 1
