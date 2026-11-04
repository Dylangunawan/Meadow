// script.name=Snowy peaks
// script.description=Rock on steep slopes, frost and stone above a height, pine forest on the lower hills.\nWritten against the WorldPainter scripting API; runs unchanged in WorldPainter and Meadow.
// script.param.snowLine.type=integer
// script.param.snowLine.description=Height above which mountains get snow
// script.param.snowLine.default=120
// script.param.forest.type=boolean
// script.param.forest.displayName=Plant pine forest
// script.param.forest.default=true

var snowLine = params['snowLine'];

// Steep slopes become rock
var steep = wp.createFilter().aboveDegrees(40).go();
wp.applyTerrain(29).toWorld(world).withFilter(steep).go();

// Snow above the snow line
var frost = wp.getLayer().withName('Frost').go();
var high = wp.createFilter().aboveLevel(snowLine).go();
wp.applyLayer(frost).toWorld(world).withFilter(high).go();

if (params['forest']) {
    var pine = wp.getLayer().withName('Pine').go();
    var hills = wp.createFilter().aboveLevel(70).belowLevel(snowLine - 10).belowDegrees(35).onlyOnLand().go();
    wp.applyLayer(pine).toWorld(world).withFilter(hills).go();
}

// The Java-style per-column API works too:
var d = world.getDimension(0);
var peaks = 0;
for (var x = d.getLowestX() * 128; x < (d.getHighestX() + 1) * 128; x += 4) {
    for (var y = d.getLowestY() * 128; y < (d.getHighestY() + 1) * 128; y += 4) {
        if (d.getHeightAt(x, y) > snowLine + 40) peaks++;
    }
}
print('Sampled ' + peaks + ' high peak columns');
