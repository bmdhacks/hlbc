interface Printable {
	public function print():String;
}

interface Countable {
	public function count():Int;
}

class Document implements Printable implements Countable {
	var content:String;

	public function new(c:String) {
		content = c;
	}

	public function print():String {
		return content;
	}

	public function count():Int {
		return content.length;
	}
}

class InterfaceTest {
	static function acceptPrintable(p:Printable):String {
		return p.print();
	}

	static function main() {
		var doc = new Document("Hello World");
		Sys.println("print=" + acceptPrintable(doc));
		Sys.println("count=" + doc.count());
	}
}
