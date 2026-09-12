package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"time"
)

// The shared request store owns content, identity, and replay fencing. A
// transport failure remains unknown; it never falls through to raw PTY input.
func (r *registry) sendCapturedRequest(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		RequestID string `json:"requestId"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" || p.RequestID == "" {
		return nil, fmt.Errorf("captured message requires sessionId and requestId")
	}
	params, _ := json.Marshal(p)
	prepared, err := r.desktopInternalCall(ctx, "internal.beginDelivery", params)
	if err != nil {
		return nil, err
	}
	if string(prepared) != "null" {
		var delivery struct {
			DeliveryID string `json:"deliveryId"`
			Text       string `json:"text"`
		}
		if err := json.Unmarshal(prepared, &delivery); err != nil {
			return nil, err
		}
		stamp := func(status string) error {
			payload, _ := json.Marshal(map[string]string{"requestId": p.RequestID, "deliveryId": delivery.DeliveryID, "status": status})
			writeCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer cancel()
			_, err := r.desktopInternalCall(writeCtx, "internal.finishDelivery", payload)
			return err
		}
		status, sendErr := r.deliverDesktopMessage(ctx, p.SessionID, delivery.Text, map[string]string{"requestId": p.RequestID, "deliveryId": delivery.DeliveryID})
		if sendErr != nil {
			status = "unknown"
		}
		if err = stamp(status); err != nil {
			return nil, err
		}
	}
	return r.desktopInternalCall(ctx, "internal.requestReceipt", params)
}

func (c *claudemonClient) deliverCaptured(ctx context.Context, id, text string) string {
	payload, _ := json.Marshal(map[string]string{"text": text})
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, c.base+"/sessions/"+url.PathEscape(id)+"/message", bytes.NewReader(payload))
	if err != nil {
		return "unknown"
	}
	request.Header.Set("Content-Type", "application/json")
	response, err := c.http.Do(request)
	if err != nil {
		return "unknown"
	}
	defer response.Body.Close()
	if response.StatusCode >= 200 && response.StatusCode < 300 {
		return "accepted"
	}
	switch response.StatusCode {
	case 400, 401, 403, 404, 409, 410, 413, 422, 429:
		return "rejected"
	case 503:
		body, _ := io.ReadAll(io.LimitReader(response.Body, 4096))
		if string(body) == "session input queue is full" {
			return "rejected"
		}
	}
	return "unknown"
}

// Route every ordinary prompt and fleet wake through the same handoff outbox.
// The transport ACK is fed back by id, so an in-flight send captured by a
// concurrent handoff remains inspectable rather than being blindly replayed.
func (r *registry) deliverDesktopMessage(ctx context.Context, id, text string, sourceRequest any) (string, error) {
	payload, _ := json.Marshal(map[string]any{"sessionId": id, "text": text, "sourceRequest": sourceRequest})
	prepared, err := r.desktopInternalCall(ctx, "internal.routeMessage", payload)
	if err != nil {
		return "unknown", err
	}
	var route struct {
		Target string `json:"target"`
		ID     string `json:"id"`
		Held   bool   `json:"held"`
	}
	if err := json.Unmarshal(prepared, &route); err != nil {
		return "unknown", err
	}
	if route.Held {
		return "pending", nil
	}
	if sourceRequest != nil {
		encoded, _ := json.Marshal(sourceRequest)
		var mark map[string]any
		_ = json.Unmarshal(encoded, &mark)
		mark["status"] = "unknown"
		encoded, _ = json.Marshal(mark)
		if _, err := r.desktopInternalCall(ctx, "internal.finishDelivery", encoded); err != nil {
			return "unknown", err
		}
	}
	status := r.cm.deliverCaptured(ctx, route.Target, text)
	result, _ := json.Marshal(map[string]string{"id": route.ID, "status": status})
	ackCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if _, err := r.desktopInternalCall(ackCtx, "internal.messageResult", result); err != nil {
		return "unknown", err
	}
	return status, nil
}
func (r *registry) submitMessage(ctx context.Context, id, text string) (bool, error) {
	if !r.desktopServices.available() {
		return r.cm.submitMessage(ctx, id, text)
	}
	status, err := r.deliverDesktopMessage(ctx, id, text, nil)
	if err != nil {
		return false, err
	}
	switch status {
	case "accepted", "pending":
		return true, nil
	case "rejected":
		return false, nil
	default:
		return false, fmt.Errorf("message acknowledgement unknown; inspect the transcript before retrying")
	}
}
