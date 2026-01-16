class TryCatch {
    static function main() {
        // Basic try-catch
        try {
            Sys.println("in try");
            throw "error1";
            Sys.println("after throw");  // should not execute
        } catch (e:Dynamic) {
            Sys.println("caught=" + e);
        }

        // Try without exception
        try {
            Sys.println("no exception");
        } catch (e:Dynamic) {
            Sys.println("should not catch");
        }

        // Nested try-catch
        try {
            try {
                throw "inner";
            } catch (e:Dynamic) {
                Sys.println("inner caught=" + e);
                throw "outer";
            }
        } catch (e:Dynamic) {
            Sys.println("outer caught=" + e);
        }

        Sys.println("done");
    }
}
