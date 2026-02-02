// API client for PeekabooVault backend

import type {
	ApiResponse,
	Camera,
	DiscoverResponse,
	ProbeResponse,
	ServerStatus,
	SetCredentialsRequest,
	SetCredentialsResponse,
	DiscoveredDevice
} from './types';

const API_BASE = '/api';

async function fetchJson<T>(url: string, options?: RequestInit): Promise<T> {
	const response = await fetch(`${API_BASE}${url}`, {
		headers: {
			'Content-Type': 'application/json',
			...options?.headers
		},
		...options
	});

	if (!response.ok) {
		throw new Error(`HTTP ${response.status}: ${response.statusText}`);
	}

	return response.json();
}

export const api = {
	// Server status
	async getStatus(): Promise<ServerStatus> {
		return fetchJson<ServerStatus>('/status');
	},

	// Discovery
	async scanDevices(durationSecs = 5): Promise<DiscoverResponse> {
		return fetchJson<DiscoverResponse>('/discovery/scan', {
			method: 'POST',
			body: JSON.stringify({ durationSecs })
		});
	},

	async getDiscoveredDevices(): Promise<ApiResponse<DiscoveredDevice[]>> {
		return fetchJson<ApiResponse<DiscoveredDevice[]>>('/discovery/devices');
	},

	// Cameras
	async listCameras(): Promise<ApiResponse<Camera[]>> {
		return fetchJson<ApiResponse<Camera[]>>('/cameras');
	},

	async addCamera(
		name: string,
		host: string,
		username?: string,
		password?: string
	): Promise<ApiResponse<Camera>> {
		return fetchJson<ApiResponse<Camera>>('/cameras', {
			method: 'POST',
			body: JSON.stringify({ name, host, username, password })
		});
	},

	async getCamera(id: string): Promise<ApiResponse<Camera>> {
		return fetchJson<ApiResponse<Camera>>(`/cameras/${id}`);
	},

	async deleteCamera(id: string): Promise<ApiResponse<Camera>> {
		return fetchJson<ApiResponse<Camera>>(`/cameras/${id}`, {
			method: 'DELETE'
		});
	},

	async setCredentials(id: string, credentials: SetCredentialsRequest): Promise<SetCredentialsResponse> {
		return fetchJson<SetCredentialsResponse>(`/cameras/${id}/credentials`, {
			method: 'POST',
			body: JSON.stringify(credentials)
		});
	},

	async probeCamera(id: string, refreshStreams = true): Promise<ProbeResponse> {
		return fetchJson<ProbeResponse>(`/cameras/${id}/probe`, {
			method: 'POST',
			body: JSON.stringify({ refreshStreams })
		});
	}
};
