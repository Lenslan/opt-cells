input state[5:0], wia_mode, origin_state;
output new_state;
one_state = (state == 6'b101011);
new_state = wia_mode & one_state | !wia_mode & origin_state;
