//! Memorable slug generation for plan files.
//!
//! Generates human-friendly slugs in the format `{adjective}-{verb}-{noun}`
//! for plan file naming. This makes plan files easy to identify and remember.

use rand::prelude::IndexedRandom;
use rand::rng;

/// Adjectives for slug generation.
const ADJECTIVES: &[&str] = &[
    // Nature/mood adjectives
    "abundant", "ancient", "bright", "calm", "cheerful", "clever", "cozy", "curious",
    "dapper", "dazzling", "deep", "delightful", "eager", "elegant", "enchanted", "fancy",
    "fluffy", "gentle", "gleaming", "golden", "graceful", "happy", "hidden", "humble",
    "jolly", "joyful", "keen", "kind", "lively", "lovely", "lucky", "luminous",
    "magical", "majestic", "mellow", "merry", "mighty", "misty", "noble", "peaceful",
    "playful", "plucky", "polished", "precious", "proud", "quiet", "quirky", "radiant",
    "rosy", "serene", "shiny", "silly", "sleepy", "smooth", "snazzy", "snug",
    "snuggly", "soft", "sparkling", "spicy", "splendid", "sprightly", "starry", "steady",
    "sunny", "swift", "tender", "tidy", "toasty", "tranquil", "twinkly", "valiant",
    "vast", "velvet", "vivid", "warm", "whimsical", "wild", "wise", "witty",
    "wondrous", "zany", "zesty", "zippy", "breezy", "bubbly", "buzzing", "cheeky",
    "cosmic", "crispy", "crystalline", "cuddly", "drifting", "dreamy", "effervescent",
    "ethereal", "fizzy", "flickering", "floating", "floofy", "fluttering", "foamy",
    "frolicking", "fuzzy", "giggly", "glimmering", "glistening", "glittery", "glowing",
    "goofy", "groovy", "harmonic", "hazy", "humming", "iridescent", "jaunty", "jazzy",
    "jiggly", "melodic", "moonlit", "mossy", "nifty", "peppy", "prancy", "purrfect",
    "purring", "quizzical", "rippling", "rustling", "sassy", "shimmering", "shimmying",
    "snappy", "snoopy", "squishy", "swirling", "ticklish", "tingly", "twinkling",
    "velvety", "wiggly", "wobbly", "woolly", "zazzy",
    // Programming adjectives
    "abstract", "adaptive", "agile", "async", "atomic", "binary", "cached", "compiled",
    "composed", "compressed", "concurrent", "cryptic", "curried", "declarative",
    "delegated", "distributed", "dynamic", "expressive", "federated", "functional",
    "generic", "greedy", "hashed", "idempotent", "immutable", "imperative", "indexed",
    "inherited", "iterative", "lazy", "lexical", "linear", "linked", "logical",
    "memoized", "modular", "mutable", "nested", "optimized", "parallel", "parsed",
    "partitioned", "piped", "polymorphic", "pure", "reactive", "recursive", "refactored",
    "reflective", "replicated", "resilient", "robust", "scalable", "sequential",
    "serialized", "sharded", "sorted", "staged", "stateful", "stateless", "streamed",
    "structured", "synchronous", "synthetic", "temporal", "transient", "typed",
    "unified", "validated", "vectorized", "virtual",
];

/// Verbs (gerund form) for slug generation.
const VERBS: &[&str] = &[
    "baking", "beaming", "booping", "bouncing", "brewing", "bubbling", "chasing",
    "churning", "coalescing", "conjuring", "cooking", "crafting", "crunching",
    "cuddling", "dancing", "dazzling", "discovering", "doodling", "dreaming",
    "drifting", "enchanting", "exploring", "finding", "floating", "fluttering",
    "foraging", "forging", "frolicking", "gathering", "giggling", "gliding",
    "greeting", "growing", "hatching", "herding", "honking", "hopping", "hugging",
    "humming", "imagining", "inventing", "jingling", "juggling", "jumping",
    "kindling", "knitting", "launching", "leaping", "mapping", "marinating",
    "meandering", "mixing", "moseying", "munching", "napping", "nibbling",
    "noodling", "orbiting", "painting", "percolating", "petting", "plotting",
    "pondering", "popping", "prancing", "purring", "puzzling", "questing",
    "riding", "roaming", "rolling", "sauteeing", "scribbling", "seeking",
    "shimmying", "singing", "skipping", "sleeping", "snacking", "sniffing",
    "snuggling", "soaring", "sparking", "spinning", "splashing", "sprouting",
    "squishing", "stargazing", "stirring", "strolling", "swimming", "swinging",
    "tickling", "tinkering", "toasting", "tumbling", "twirling", "waddling",
    "wandering", "watching", "weaving", "whistling", "wibbling", "wiggling",
    "wishing", "wobbling", "wondering", "yawning", "zooming",
];

/// Nouns for slug generation.
const NOUNS: &[&str] = &[
    // Nature
    "aurora", "avalanche", "blossom", "breeze", "brook", "bubble", "canyon",
    "cascade", "cloud", "clover", "comet", "coral", "cosmos", "creek", "crescent",
    "crystal", "dawn", "dewdrop", "dusk", "eclipse", "ember", "feather", "fern",
    "firefly", "flame", "flurry", "fog", "forest", "frost", "galaxy", "garden",
    "glacier", "glade", "grove", "harbor", "horizon", "island", "lagoon", "lake",
    "leaf", "lightning", "meadow", "meteor", "mist", "moon", "moonbeam", "mountain",
    "nebula", "nova", "ocean", "orbit", "pebble", "petal", "pine", "planet",
    "pond", "puddle", "quasar", "rain", "rainbow", "reef", "ripple", "river",
    "shore", "sky", "snowflake", "spark", "spring", "star", "stardust", "starlight",
    "storm", "stream", "summit", "sun", "sunbeam", "sunrise", "sunset", "thunder",
    "tide", "twilight", "valley", "volcano", "waterfall", "wave", "willow", "wind",
    // Animals
    "alpaca", "axolotl", "badger", "bear", "beaver", "bee", "bird", "bumblebee",
    "bunny", "butterfly", "capybara", "cat", "chipmunk", "crab", "crane", "deer",
    "dolphin", "dove", "dragon", "dragonfly", "duck", "duckling", "eagle",
    "elephant", "falcon", "finch", "flamingo", "fox", "frog", "giraffe", "goose",
    "hamster", "hare", "hedgehog", "hippo", "hummingbird", "jellyfish", "kitten",
    "koala", "ladybug", "lark", "lemur", "llama", "lobster", "lynx", "manatee",
    "meerkat", "moth", "narwhal", "newt", "octopus", "otter", "owl", "panda",
    "parrot", "peacock", "pelican", "penguin", "phoenix", "piglet", "platypus",
    "pony", "porcupine", "puffin", "puppy", "quail", "quokka", "rabbit", "raccoon",
    "raven", "robin", "salamander", "seahorse", "seal", "sloth", "snail", "sparrow",
    "sphinx", "squid", "squirrel", "starfish", "starling", "swan", "tiger",
    "toucan", "turtle", "unicorn", "walrus", "whale", "wolf", "wombat", "wren",
    "yeti", "zebra",
    // Objects
    "acorn", "anchor", "balloon", "beacon", "biscuit", "blanket", "bonbon", "book",
    "boot", "button", "cake", "candle", "candy", "castle", "charm", "clock",
    "cocoa", "compass", "cookie", "crayon", "crown", "cupcake", "donut", "dream",
    "fairy", "fiddle", "flask", "flute", "fountain", "gadget", "gem", "gizmo",
    "globe", "goblet", "hammock", "harp", "haven", "hearth", "honey", "jingle",
    "journal", "kazoo", "kettle", "key", "kite", "lantern", "lemon", "lighthouse",
    "locket", "lollipop", "mango", "map", "marble", "marshmallow", "melody",
    "mitten", "mochi", "muffin", "music", "nest", "noodle", "oasis", "origami",
    "pancake", "parasol", "peach", "pearl", "pie", "pillow", "pinwheel", "pixel",
    "pizza", "plum", "popcorn", "pretzel", "prism", "pudding", "pumpkin", "puzzle",
    "quiche", "quill", "quilt", "riddle", "rocket", "rose", "scone", "scroll",
    "shell", "sketch", "snowglobe", "sonnet", "sparkle", "spindle", "sprout",
    "sundae", "swing", "taco", "teacup", "teapot", "thimble", "toast", "token",
    "tome", "tower", "treasure", "treehouse", "trinket", "truffle", "tulip",
    "umbrella", "waffle", "wand", "whisper", "whistle", "widget", "wreath", "zephyr",
    // CS pioneers
    "abelson", "adleman", "aho", "allen", "babbage", "bachman", "backus", "barto",
    "bengio", "bentley", "blum", "boole", "brooks", "catmull", "cerf", "cherny",
    "church", "clarke", "cocke", "codd", "conway", "cook", "corbato", "cray",
    "curry", "dahl", "diffie", "dijkstra", "dongarra", "eich", "emerson",
    "engelbart", "feigenbaum", "floyd", "gehret", "goldwasser", "gosling", "graham",
    "gray", "hamming", "hanrahan", "hartmanis", "hejlsberg", "hellman", "hennessy",
    "hickey", "hinton", "hoare", "hollerith", "hopcroft", "hopper", "iverson",
    "kahan", "kahn", "karp", "kay", "kernighan", "knuth", "kurzweil", "lamport",
    "lampson", "lecun", "lerdorf", "liskov", "lovelace", "matsumoto", "mccarthy",
    "metcalfe", "micali", "milner", "minsky", "moler", "moore", "naur", "neumann",
    "newell", "nygaard", "papert", "parnas", "pascal", "patterson", "pearl",
    "perlis", "pike", "pnueli", "rabin", "reddy", "ritchie", "rivest", "rossum",
    "russell", "scott", "sedgewick", "shamir", "shannon", "sifakis", "simon",
    "stallman", "stearns", "steele", "stonebraker", "stroustrup", "sutherland",
    "sutton", "tarjan", "thacker", "thompson", "torvalds", "turing", "ullman",
    "valiant", "wadler", "wall", "wigderson", "wilkes", "wilkinson", "wirth",
    "wozniak", "yao",
];

/// Generate a random memorable slug.
///
/// Format: `{adjective}-{verb}-{noun}`
/// Example: "atomic-marinating-pumpkin"
pub fn generate_slug() -> String {
    let mut rng = rng();

    let adjective = ADJECTIVES.choose(&mut rng).unwrap_or(&"random");
    let verb = VERBS.choose(&mut rng).unwrap_or(&"coding");
    let noun = NOUNS.choose(&mut rng).unwrap_or(&"plan");

    format!("{adjective}-{verb}-{noun}")
}

/// Total number of possible unique slugs.
#[allow(dead_code)]
pub fn total_combinations() -> usize {
    ADJECTIVES.len() * VERBS.len() * NOUNS.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_slug_format() {
        let slug = generate_slug();
        let parts: Vec<&str> = slug.split('-').collect();
        assert_eq!(parts.len(), 3, "Slug should have 3 parts: {slug}");
    }

    #[test]
    fn test_slugs_are_different() {
        let slug1 = generate_slug();
        let slug2 = generate_slug();
        // With millions of combinations, collision is extremely unlikely
        assert_ne!(slug1, slug2, "Two random slugs should differ");
    }

    #[test]
    fn test_total_combinations() {
        let total = total_combinations();
        // Should have millions of combinations
        assert!(total > 1_000_000, "Should have many combinations: {total}");
    }
}
