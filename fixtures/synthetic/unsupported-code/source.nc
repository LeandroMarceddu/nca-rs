(state-changing unsupported construct stops geometry)
G21 G90 G54 G17
G0 Z5
G0 X0 Y0
M3 S10000
G1 Z-1 F150
G1 X10 F700
G41 D1
G1 X20
G1 Y10
M5
M30
