# Inputs regression frame

`factory-375-inputs.png` is the unmodified screenshot from the failed WebGPU
smoke job in https://github.com/petitstrawberry/scarlet-ui/actions/runs/42313230307
(main `eb6ce5b`, PR #82). It contains only the checked-in Widget Factory sample.
The old x+200 scan splits TextView at “Edit headings and TODO markers.” and
reports four surfaces instead of three. Keep the complete frame to exercise
the real text rasterization, labels, slider and navigation around the surfaces.
