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
		Sys.println("Red=" + colorName(Red));
		Sys.println("Green=" + colorName(Green));
		Sys.println("Blue=" + colorName(Blue));
		Sys.println("Custom=" + colorName(Custom(128, 64, 32)));

		Sys.println("isWarm Red=" + isWarm(Red));
		Sys.println("isWarm Blue=" + isWarm(Blue));
		Sys.println("isWarm Custom=" + isWarm(Custom(255, 0, 0)));

		var rgb = toRgb(Green);
		Sys.println("toRgb Green=" + rgb[0] + "," + rgb[1] + "," + rgb[2]);
	}
}
