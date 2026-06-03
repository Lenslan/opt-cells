create_cell     eco_INR4D0BWP7T40P140_u1        [get_lib_cells */INR4D0BWP7T40P140]
create_net      eco_n9_u1
connect_net     a                               [get_pin eco_INR4D0BWP7T40P140_u1/A1]
connect_net     b                               [get_pin eco_INR4D0BWP7T40P140_u1/B1]
connect_net     c                               [get_pin eco_INR4D0BWP7T40P140_u1/B2]
connect_net     d                               [get_pin eco_INR4D0BWP7T40P140_u1/B3]
connect_net     eco_n9_u1                       [get_pin eco_INR4D0BWP7T40P140_u1/ZN]

create_cell     eco_AN3D0BWP7T40P140_u0         [get_lib_cells */eco_AN3D0BWP7T40P140]
connect_net     e                               [get_pin eco_AN3D0BWP7T40P140/A1]
connect_net     f                               [get_pin eco_AN3D0BWP7T40P140/A2]
connect_net     eco_n9_u1                       [get_pin eco_AN3D0BWP7T40P140/A3]

connect_net     one_state                       [get_pin eco_AN3D0BWP7T40P140/Z]