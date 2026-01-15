// Tests enum definition and switch on enum values
// Bug: decompiler uses numeric indices instead of enum names
// Original: case Red: "red"
// Decompiled: case 0: "red"

enum Color {
	Red;
	Green;
	Blue;
	Custom(r:Int, g:Int, b:Int);
}

class EnumSwitch {
	static function colorName(c:Color):String {
		return switch (c) {
			case Red: "red";
			case Green: "green";
			case Blue: "blue";
			case Custom(r, g, b): "custom(" + r + "," + g + "," + b + ")";
		};
	}

	static function isWarm(c:Color):Bool {
		return switch (c) {
			case Red: true;
			case Custom(r, _, _) if (r > 200): true;
			default: false;
		};
	}

	static function toRgb(c:Color):Array<Int> {
		return switch (c) {
			case Red: [255, 0, 0];
			case Green: [0, 255, 0];
			case Blue: [0, 0, 255];
			case Custom(r, g, b): [r, g, b];
		};
	}

	static function main() {
		trace("Red=" + colorName(Red));
		trace("Green=" + colorName(Green));
		trace("Blue=" + colorName(Blue));
		trace("Custom=" + colorName(Custom(128, 64, 32)));

		trace("isWarm Red=" + isWarm(Red));
		trace("isWarm Blue=" + isWarm(Blue));
		trace("isWarm Custom=" + isWarm(Custom(255, 0, 0)));

		var rgb = toRgb(Green);
		trace("toRgb Green=" + rgb[0] + "," + rgb[1] + "," + rgb[2]);
	}
}
