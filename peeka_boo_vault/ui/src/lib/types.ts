// API types matching the Rust backend

export interface DiscoveredDevice {
	id: string;
	address: string;
	name: string | null;
	hardware: string | null;
	types: string[];
	urls: string[];
	discoveredAt: string;
	isOnboarded: boolean;
}

export interface Camera {
	id: string;
	name: string;
	onvifAddress: string | null;
	onvifUrl: string | null;
	mainStreamUri: string | null;
	subStreamUri: string | null;
	streamProfiles: StreamProfile[];
	hasCredentials: boolean;
	status: CameraStatus;
	lastSeen: string | null;
	manufacturer: string | null;
	model: string | null;
	firmwareVersion: string | null;
	serialNumber: string | null;
	hardwareId: string | null;
}

export type CameraStatus = 'online' | 'offline' | 'unauthorized' | 'unknown' | 'probing';

export interface DiscoverRequest {
	durationSecs?: number;
}

export interface DiscoverResponse {
	success: boolean;
	error: string | null;
	devices: DiscoveredDevice[];
	scanDurationMs: number;
}

export interface AddCameraRequest {
	name: string;
	onvifAddress?: string;
	onvifUrl: string;
}

export interface SetCredentialsRequest {
	username: string;
	password: string;
}

export interface SetCredentialsResponse {
	success: boolean;
	error: string | null;
	camera: Camera | null;
}

export interface ProbeRequest {
	refreshStreams?: boolean;
}

export interface ProbeResponse {
	success: boolean;
	error: string | null;
	camera: Camera | null;
	profiles: StreamProfile[];
}

export interface StreamProfile {
	token: string;
	name: string;
	streamUri: string;
	encoding: string | null;
	width: number | null;
	height: number | null;
	frameRate: number | null;
	bitrateKbps: number | null;
	quality: number | null;
	gopLength: number | null;
	codecProfile: string | null;
	encodingInterval: number | null;
	guaranteedFrameRate: boolean | null;
}

export interface ApiResponse<T> {
	success: boolean;
	error: string | null;
	data: T | null;
}

export interface ServerStatus {
	version: string;
	uptimeSecs: number;
	cameraCount: number;
	camerasOnline: number;
	discoveryInProgress: boolean;
}
