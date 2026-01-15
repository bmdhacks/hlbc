// Tests constructor calls with computed inline arguments
// Bug: decompiler may output variables before they're computed
// Original: new Point(a * b, c + d)
// Decompiled: new Point(r3, r4); r3 = a * b; r4 = c + d;  // Wrong order!

class Vec2 {
	public var x:Float;
	public var y:Float;

	public function new(x:Float, y:Float) {
		this.x = x;
		this.y = y;
	}

	public function toString():String {
		return "(" + x + "," + y + ")";
	}
}

class ConstructorArgs {
	static function add(a:Vec2, b:Vec2):Vec2 {
		return new Vec2(a.x + b.x, a.y + b.y);
	}

	static function scale(v:Vec2, s:Float):Vec2 {
		return new Vec2(v.x * s, v.y * s);
	}

	static function dot(a:Vec2, b:Vec2):Float {
		return a.x * b.x + a.y * b.y;
	}

	static function perpendicular(v:Vec2):Vec2 {
		return new Vec2(-v.y, v.x);
	}

	static function lerp(a:Vec2, b:Vec2, t:Float):Vec2 {
		return new Vec2(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
	}

	static function fromAngle(angle:Float):Vec2 {
		return new Vec2(Math.cos(angle), Math.sin(angle));
	}

	static function main() {
		var a = new Vec2(1.0, 2.0);
		var b = new Vec2(3.0, 4.0);

		var sum = add(a, b);
		trace("add=" + sum.toString());

		var scaled = scale(a, 2.0);
		trace("scale=" + scaled.toString());

		trace("dot=" + dot(a, b));

		var perp = perpendicular(a);
		trace("perp=" + perp.toString());

		var mid = lerp(a, b, 0.5);
		trace("lerp=" + mid.toString());

		var unit = fromAngle(0.0);
		trace("fromAngle x=" + unit.x);
	}
}
