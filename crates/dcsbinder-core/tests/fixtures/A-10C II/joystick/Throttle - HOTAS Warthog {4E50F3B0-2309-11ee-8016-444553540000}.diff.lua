local diff = {
	["axisDiffs"] = {
		["a2033cdnil"] = {
			["changed"] = {
				[1] = {
					["filter"] = {
						["curvature"] = {
							[1] = 0.3,
						},
						["deadzone"] = 0.15,
						["hardwareDetent"] = false,
						["hardwareDetentAB"] = 0,
						["hardwareDetentMax"] = 0,
						["invert"] = false,
						["saturationX"] = 1,
						["saturationY"] = 1,
						["slider"] = false,
					},
					["key"] = "JOY_X",
				},
			},
			["name"] = "HOTAS Slew Horizontal",
		},
		["a2034cdnil"] = {
			["changed"] = {
				[1] = {
					["filter"] = {
						["curvature"] = {
							[1] = 0.3,
						},
						["deadzone"] = 0.15,
						["hardwareDetent"] = false,
						["hardwareDetentAB"] = 0,
						["hardwareDetentMax"] = 0,
						["invert"] = true,
						["saturationX"] = 1,
						["saturationY"] = 1,
						["slider"] = false,
					},
					["key"] = "JOY_Y",
				},
			},
			["name"] = "HOTAS Slew Vertical",
		},
	},
	["keyDiffs"] = {
		["d1047pnilu1568cdnilvdnilvpnilvunil"] = {
			["name"] = "Flaps : Up<>Center",
			["removed"] = {
				[1] = {
					["key"] = "JOY_BTN22",
				},
			},
		},
		["d1049pnilu1569cdnilvdnilvpnilvunil"] = {
			["name"] = "Flaps : Down<>Center",
			["removed"] = {
				[1] = {
					["key"] = "JOY_BTN23",
				},
			},
		},
		["d572pnilu576cdnilvdnilvpnilvunil"] = {
			["name"] = "HOTAS MIC Switch Up",
			["removed"] = {
				[1] = {
					["key"] = "JOY_BTN3",
				},
			},
		},
		["d573pnilu576cdnilvdnilvpnilvunil"] = {
			["name"] = "HOTAS MIC Switch Down (call radio menu)",
			["removed"] = {
				[1] = {
					["key"] = "JOY_BTN5",
				},
			},
		},
		["d574pnilu576cdnilvdnilvpnilvunil"] = {
			["name"] = "HOTAS MIC Switch Aft (call radio menu)",
			["removed"] = {
				[1] = {
					["key"] = "JOY_BTN6",
				},
			},
		},
		["d575pnilu576cdnilvdnilvpnilvunil"] = {
			["name"] = "HOTAS MIC Switch Forward (call radio menu)",
			["removed"] = {
				[1] = {
					["key"] = "JOY_BTN4",
				},
			},
		},
	},
}
return diff