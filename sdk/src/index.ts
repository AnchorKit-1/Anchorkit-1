// Main entry point for @anchorkit/sdk

export { Sep10Flow } from './components/Sep10Flow/Sep10Flow';
export { Sep10Service, Sep10ServiceError } from './services/sep10';
export type { Sep10FlowProps } from './components/Sep10Flow/Sep10Flow';
export type {
  Sep10Stage,
  Sep10State,
  Sep10Config,
  ChallengeResponse,
  AuthResponse,
  Sep10Error,
} from './types/sep10';

// SEP-6 streaming
export { Sep6StreamingService, Sep6StreamError } from './services/sep6Streaming';
export { TransactionStream } from './services/sep6Stream';
export {
  isTerminalStatus,
  TERMINAL_STATUSES,
} from './types/sep6';
export type {
  Sep6TransactionStatus,
  Sep6Transaction,
  Sep6TransactionResponse,
  Sep6TransactionsResponse,
  Sep6StreamConfig,
  Sep6StatusUpdate,
  StreamCloseEvent,
  StreamCloseReason,
  StreamMode,
  TransactionStreamEvent,
  TransactionStreamError,
  StreamHandle,
  WatcherState,
  AnchorStreamCapability,
} from './types/sep6';
