-- A junction with four arms. OFFSET gives each replica different entity ids.
-- Constructors match the shipped API; only command execution is a stand-in.
local off = OFFSET or 0
local function id(n) return n+off end
local CT = api.type.ComponentType
NODES, EDGES, STREETS, CONFIGS = {}, {}, {}, {}
NODES[id(1)] = {x=0,y=0,z=0}
local locations = {{100,0}, {0,100}, {-100,0}, {0,-100}}
local function lane(forward)
	return {speed=13.5,width=3.5,height=0,offset=forward and 1.75 or -1.75,
		forward=forward,transportModes={[0]=true,[1]=true,[2]=true}}
end
for i,p in ipairs(locations) do
	NODES[id(i+1)] = {x=p[1],y=p[2],z=0}
	EDGES[id(100+i)] = {node0=id(1),node1=id(i+1), position0=NODES[id(1)],position1=NODES[id(i+1)],
		tangent0={x=p[1],y=p[2],z=0},tangent1={x=p[1],y=p[2],z=0},
		roadTemplate='::/street/town_small.street_template',laneConfigs={lane(true),lane(false)},objects={}}
	STREETS[id(i+1)] = {id(100+i)}
end
STREETS[id(1)] = {id(101),id(102),id(103),id(104)}
api.res.trafficLightTypeRep = {
	getName=function(n) assert(n==id(40)) return '::/traffic_light/standard.lua' end,
	find=function(s) return s=='::/traffic_light/standard.lua' and id(40) or -1 end,
}
api.engine.system.streetSystem.getNode2SegmentMap = function() return STREETS end
CONFIGS[id(1)] = {laneConnections={
	{segment0=id(101),lane0=1,segment1=id(102),lane1=0,withRoad=true,withTram=false},
	{segment0=id(103),lane0=1,segment1=id(104),lane1=0,withRoad=true,withTram=true},
},crosswalks={id(101),id(103)},trafficLightPreference=1,
	trafficLightConfig={trafficLightType=id(40),states={
		{lockedLanes={0,2},duration=12.375,minDuration=4.125,canSkip=true},
		{lockedLanes={1,3},duration=20,minDuration=8,canSkip=false},
	}},doubleSlipSwitch=false,userModifiedTrafficLightStates=true}
PROPOSAL = {toAdd={},toRemove={},proposal={nodeConfigsToAdd={{entity=id(1),comp=CONFIGS[id(1)]}},
	nodeConfigsToRemove={id(1)}}}
J = ug_require('tpf3mp_1::/scripts/tpf3mp/junctions.lua')
J.strict_junctions = true
C = ug_require('tpf3mp_1::/scripts/tpf3mp/capture.lua')
A = ug_require('tpf3mp_1::/scripts/tpf3mp/apply.lua')
