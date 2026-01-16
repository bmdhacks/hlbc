class OperatorPrecedence {
	static function main() {
		// Division precedence (found in h2d/Object.hx:495)
		var a = 2.0;
		var b = 3.0;
		var c = 4.0;
		var result1 = 1 / (a * b - c);
		Sys.println("div=" + result1);

		// Bitwise AND with comparison (found in h2d/Bitmap.hx:19)
		var flags = 17;
		var hasBit = (flags & 16) != 0;
		Sys.println("bit=" + hasBit);

		// Shift and mask (found in h3d/Engine.hx:659)
		var color = 0xFF8040;
		var r = (color >> 16) & 255;
		var g = (color >> 8) & 255;
		var b = color & 255;
		Sys.println("rgb=" + r + "," + g + "," + b);

		// Compound precedence
		var x = 5;
		var y = 3;
		var z = 2;
		var compound = x + y * z;
		Sys.println("compound=" + compound);

		// Nested parentheses
		var nested = (x + y) * (z + 1);
		Sys.println("nested=" + nested);
	}
}
