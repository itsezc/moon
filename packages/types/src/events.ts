import type { Duration, Runtime } from './common';
import type { Project, Task } from './project';
import type { Action, ActionNode } from './runner';

export interface ProviderEnvironment {
	baseBranch: string | null;
	branch: string;
	id: string;
	provider: string;
	requestId: string | null;
	requestUrl: string | null;
	revision: string;
	url: string | null;
}

export interface WebhookPayload<T extends EventType, E> {
	createdAt: string;
	environment: ProviderEnvironment | null;
	event: E;
	type: T;
	uuid: string;
}

export type EventType =
	| 'action.finished'
	| 'action.started'
	| 'dependencies.installed'
	| 'dependencies.installing'
	| 'project.synced'
	| 'project.syncing'
	| 'runner.aborted'
	| 'runner.finished'
	| 'runner.started'
	| 'target-output.archived'
	| 'target-output.archiving'
	| 'target-output.cache-check'
	| 'target-output.hydrated'
	| 'target-output.hydrating'
	| 'target.ran'
	| 'target.running'
	| 'tool.installed'
	| 'tool.installing';

export interface EventActionStarted {
	action: Action;
	node: ActionNode;
}

export type PayloadActionStarted = WebhookPayload<'action.started', EventActionStarted>;

export interface EventActionFinished {
	action: Action;
	error: string | null;
	node: ActionNode;
}

export type PayloadActionFinished = WebhookPayload<'action.finished', EventActionFinished>;

export interface EventDependenciesInstalling {
	project: Project | null;
	runtime: Runtime;
}

export type PayloadDependenciesInstalling = WebhookPayload<
	'dependencies.installing',
	EventDependenciesInstalling
>;

export interface EventDependenciesInstalled {
	error: string | null;
	project: Project | null;
	runtime: Runtime;
}

export type PayloadDependenciesInstalled = WebhookPayload<
	'dependencies.installed',
	EventDependenciesInstalled
>;

export interface EventProjectSyncing {
	project: Project;
	runtime: Runtime;
}

export type PayloadProjectSyncing = WebhookPayload<'project.syncing', EventProjectSyncing>;

export interface EventProjectSynced {
	error: string | null;
	project: Project;
	runtime: Runtime;
}

export type PayloadProjectSynced = WebhookPayload<'project.synced', EventProjectSynced>;

export interface EventRunnerAborted {
	error: string;
}

export type PayloadRunnerAborted = WebhookPayload<'runner.aborted', EventRunnerAborted>;

export interface EventRunnerStarted {
	actionsCount: number;
}

export type PayloadRunnerStarted = WebhookPayload<'runner.started', EventRunnerStarted>;

export interface EventRunnerFinished {
	duration: Duration;
	cachedCount: number;
	failedCount: number;
	passedCount: number;
}

export type PayloadRunnerFinished = WebhookPayload<'runner.finished', EventRunnerFinished>;

export interface EventTargetRunning {
	target: string;
}

export type PayloadTargetRunning = WebhookPayload<'target.running', EventTargetRunning>;

export interface EventTargetRan {
	error: string | null;
	target: string;
}

export type PayloadTargetRan = WebhookPayload<'target.ran', EventTargetRan>;

export interface EventTargetOutputArchiving {
	hash: string;
	project: Project;
	target: string;
	task: Task;
}

export type PayloadTargetOutputArchiving = WebhookPayload<
	'target-output.archiving',
	EventTargetOutputArchiving
>;

export interface EventTargetOutputArchived {
	archivePath: string;
	hash: string;
	project: Project;
	target: string;
	task: Task;
}

export type PayloadTargetOutputArchived = WebhookPayload<
	'target-output.archived',
	EventTargetOutputArchived
>;

export interface EventTargetOutputHydrating {
	hash: string;
	project: Project;
	target: string;
	task: Task;
}

export type PayloadTargetOutputHydrating = WebhookPayload<
	'target-output.hydrating',
	EventTargetOutputHydrating
>;

export interface EventTargetOutputHydrated {
	archivePath: string;
	hash: string;
	project: Project;
	target: string;
	task: Task;
}

export type PayloadTargetOutputHydrated = WebhookPayload<
	'target-output.hydrated',
	EventTargetOutputHydrated
>;

export interface EventTargetOutputCacheCheck {
	hash: string;
	target: string;
}

export type PayloadTargetOutputCacheCheck = WebhookPayload<
	'target-output.cache-check',
	EventTargetOutputCacheCheck
>;

export interface EventToolInstalling {
	runtime: Runtime;
}

export type PayloadToolInstalling = WebhookPayload<'tool.installing', EventToolInstalling>;

export interface EventToolInstalled {
	error: string | null;
	runtime: Runtime;
}

export type PayloadToolInstalled = WebhookPayload<'tool.installed', EventToolInstalled>;
