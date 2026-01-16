class PropertyAccess {
	var _value:Int = 0;
	var _readCount:Int = 0;

	public var value(get, set):Int;

	function get_value():Int {
		_readCount++;
		return _value;
	}

	function set_value(v:Int):Int {
		_value = v * 2;
		return _value;
	}

	public var readCount(get, never):Int;

	function get_readCount():Int {
		return _readCount;
	}

	public function new() {}

	static function main() {
		var obj = new PropertyAccess();
		obj.value = 5;
		var v = obj.value;
		var v2 = obj.value;
		Sys.println("value=" + v + " reads=" + obj.readCount);
	}
}
