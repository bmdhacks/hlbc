class Counter {
	public static var instanceCount:Int = 0;
	public static var MULTIPLIER:Int = 10;

	var id:Int;

	public function new() {
		instanceCount++;
		id = instanceCount;
	}

	public static function getTotal():Int {
		return instanceCount * MULTIPLIER;
	}

	public function getId():Int {
		return id;
	}
}

class StaticMembers {
	static function main() {
		var a = new Counter();
		var b = new Counter();
		var c = new Counter();
		Sys.println("instances=" + Counter.instanceCount);
		Sys.println("total=" + Counter.getTotal());
		Sys.println("ids=" + a.getId() + "," + b.getId() + "," + c.getId());
	}
}
