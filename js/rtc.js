export async function init_rtc_client(rtc, ws, hostName, selfName) {
    setUpIce(rtc, ws, selfName, hostName);

    await new Promise((resolve) => ws.addEventListener('open', () => resolve(), {once: true}));

    let offer = await rtc.createOffer();
    await rtc.setLocalDescription(offer);

    ws.send(JSON.stringify({
        protocol: "one-to-one",
        to: hostName,
        from: selfName,
        type: "rtc-offer",
        payload: offer
    }));

    // receive answer

    const mkListener = (resolve) => {
        function listener(message) {
            let parsed = JSON.parse(message.data);
            console.log('ws listener for rtc answer firing');
            if (parsed && parsed.from === hostName && parsed.type === "rtc-answer" && parsed.payload) {
                resolve(parsed.payload);
                rmListener();
            }
        }

        function rmListener() {
            console.log('removing ws listener for rtc answer');
            ws.removeEventListener('message', listener);
        }


        ws.addEventListener('message', listener);
    };

    let answer = await new Promise(resolve => mkListener(resolve));

    await rtc.setRemoteDescription(answer);
}

export async function init_rtc_host(rtc, ws, hostName, clientName, offer) {
    setUpIce(rtc, ws, hostName, clientName);
    await new Promise((resolve) => ws.addEventListener('open', () => resolve(), {once: true}));

    await rtc.setRemoteDescription(offer);

    let answer = await rtc.createAnswer();
    await rtc.setLocalDescription(answer);

    ws.send(JSON.stringify({
        protocol: "one-to-one",
        to: clientName,
        from: hostName,
        type: "rtc-answer",
        payload: answer
    }))
}

function setUpIce(rtc, ws, local, remote) {
    rtc.addEventListener('icecandidate', e => {
        if (e.candidate) {
            ws.send(JSON.stringify({
                protocol: "one-to-one",
                to: remote,
                from: local,
                type: "ice-candidate",
                payload: e.candidate
            }));
        }
    });

    ws.addEventListener('message', e => {
        let parsed = JSON.parse(e.data);
        if (parsed && parsed.from && parsed.from === remote && parsed.type && parsed.type === "ice-candidate") {
            rtc.addIceCandidate(parsed.payload);
        }
    });
}