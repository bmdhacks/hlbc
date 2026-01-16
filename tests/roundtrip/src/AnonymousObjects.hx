class AnonymousObjects {
	static function getPoint():{x:Int, y:Int} {
		return {x: 10, y: 20};
	}

	static function distance(p1:{x:Int, y:Int}, p2:{x:Int, y:Int}):Float {
		var dx = p2.x - p1.x;
		var dy = p2.y - p1.y;
		return Math.sqrt(dx * dx + dy * dy);
	}

	static function main() {
		var p1 = getPoint();
		var p2 = {x: 13, y: 24};
		Sys.println("p1=" + p1.x + "," + p1.y);
		Sys.println("dist=" + distance(p1, p2));
	}
}
