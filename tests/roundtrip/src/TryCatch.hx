class TryCatch {
    static function main() {
        // Basic try-catch
        try {
            trace("in try");
            throw "error1";
            trace("after throw");  // should not execute
        } catch (e:Dynamic) {
            trace("caught=" + e);
        }

        // Try without exception
        try {
            trace("no exception");
        } catch (e:Dynamic) {
            trace("should not catch");
        }

        // Nested try-catch
        try {
            try {
                throw "inner";
            } catch (e:Dynamic) {
                trace("inner caught=" + e);
                throw "outer";
            }
        } catch (e:Dynamic) {
            trace("outer caught=" + e);
        }

        trace("done");
    }
}
