import yfinance as yf
import requests
import os
import warnings
import datetime
from binance.client import Client
warnings.filterwarnings('ignore')

TELEGRAM_TOKEN = os.environ.get('TELEGRAM_TOKEN')
CHAT_ID = os.environ.get('CHAT_ID')
RISK_PER_TRADE = 1000 

BINANCE_API_KEY = os.environ.get('BINANCE_API_KEY')
BINANCE_SECRET_KEY = os.environ.get('BINANCE_SECRET_KEY')

def send_telegram_alert(message):
    if not TELEGRAM_TOKEN or not CHAT_ID: return
    url = f"https://api.telegram.org/bot{TELEGRAM_TOKEN}/sendMessage"
    payload = {"chat_id": CHAT_ID, "text": message}
    try:
        requests.post(url, data=payload)
    except Exception: pass

# --- BINANCE CONNECTION TEST ---
client = None
try:
    if not BINANCE_API_KEY or not BINANCE_SECRET_KEY:
        raise Exception("API Keys GitHub Secrets mein nahi mili! (Name check karo)")
    else:
        client = Client(BINANCE_API_KEY, BINANCE_SECRET_KEY, testnet=True)
        send_telegram_alert("✅ Binance Testnet Connection Successful!")
except Exception as e:
    send_telegram_alert(f"⚠️ Binance Connection FAILED!\nReason: {e}")

def check_wallet():
    if client is None: return
    try:
        account = client.get_account()
        total_usdt_value = 0.0
        wallet_msg = "💼 **WALLET UPDATE** 💼\n\n"
        for balance in account['balances']:
            asset = balance['asset']
            free = float(balance['free'])
            if free > 0:
                if asset == "USDT":
                    total_usdt_value += free
                    wallet_msg += f"💰 {asset}: {round(free, 2)}\n"
                else:
                    try:
                        ticker = client.get_symbol_ticker(symbol=f"{asset}USDT")
                        price = float(ticker['price'])
                        value_in_usdt = free * price
                        total_usdt_value += value_in_usdt
                        wallet_msg += f"🪙 {asset}: {round(free, 4)} (≈ ${round(value_in_usdt, 2)})\n"
                    except:
                        wallet_msg += f"🪙 {asset}: {round(free, 4)}\n"
        profit_loss = total_usdt_value - 10000
        wallet_msg += f"\n📊 **Total Value:** ≈ ${round(total_usdt_value, 2)}\n"
        if profit_loss >= 0:
            wallet_msg += f"🟢 **Total P&L:** +${round(profit_loss, 2)}"
        else:
            wallet_msg += f"🔴 **Total P&L:** -${round(abs(profit_loss), 2)}"
        send_telegram_alert(wallet_msg)
    except Exception as e:
        send_telegram_alert(f"⚠️ Wallet check error: {e}")

def place_binance_order(symbol, side):
    if client is None:
        return "❌ Order Failed: Binance connect nahi hai"
    try:
        if side == "BUY":
            order = client.order_market_buy(symbol=symbol, quantity=0.001)
            return f"✅ TESTNET BUY Success! ID: {order['orderId']}"
        elif side == "SELL":
            order = client.order_market_sell(symbol=symbol, quantity=0.001)
            return f"✅ TESTNET SELL Success! ID: {order['orderId']}"
    except Exception as e:
        return f"❌ Order Failed: {e}"
    return ""

def scan_market(ticker, name, interval, period, style):
    try:
        d = yf.download(ticker, period=period, interval=interval, progress=False).dropna()
        if len(d) < 200: return 

        close = d["Close"].squeeze()
        open_p = d["Open"].squeeze()
        high = d["High"].squeeze()
        low = d["Low"].squeeze()
        volume = d["Volume"].squeeze()
        
        ema9 = close.ewm(span=9).mean().iloc[-1]
        ema21 = close.ewm(span=21).mean().iloc[-1]
        ema200 = close.ewm(span=200).mean().iloc[-1] 
        
        delta = close.diff()
        gain = delta.clip(lower=0).rolling(14).mean().iloc[-1]
        loss = -delta.clip(upper=0).rolling(14).mean().iloc[-1]
        rsi = 100 - (100 / (1 + gain/loss)) if loss != 0 else 100
        
        macd_line = close.ewm(span=12).mean() - close.ewm(span=26).mean()
        signal_line = macd_line.ewm(span=9).mean()
        macd_bullish = macd_line.iloc[-1] > signal_line.iloc[-1]
        
        tr = high - low
        atr = tr.rolling(14).mean().iloc[-1]
        
        price = round(float(close.iloc[-1]), 2)
        rsi = round(float(rsi), 1)

        current_vol = volume.iloc[-1]
        avg_vol = volume.rolling(20).mean().iloc[-1]
        vol_ratio = current_vol / avg_vol if avg_vol != 0 else 1
        
        recent_high = high.tail(30).max()
        recent_low = low.tail(30).min()
        near_resistance = price >= (recent_high * 0.995)
        near_support = price <= (recent_low * 1.005)

        if ema9 > ema21 and price > ema200 and macd_bullish and rsi < 70 and vol_ratio >= 0.5:
            if not near_resistance:
                sl = round(price - (atr * 1.5), 2)
                tp = round(price + (atr * 3.0), 2)
                order_status = ""
                if ".NS" not in ticker and ticker != "GC=F":
                    binance_symbol = ticker.replace("-USD", "USDT")
                    order_status = place_binance_order(binance_symbol, "BUY")
                
                msg = f"🟢 BUY SIGNAL 🟢\nAsset: {name} ({style})\nPrice: {price}\n{order_status}\n🛑 SL: {sl} | 🎯 TP: {tp}"
                send_telegram_alert(msg) 
            
        elif ema9 < ema21 and price < ema200 and not macd_bullish and vol_ratio >= 0.5:
            if not (near_support and rsi < 35) and rsi > 30:
                sl = round(price + (atr * 1.5), 2)
                tp = round(price - (atr * 3.0), 2)
                order_status = ""
                if ".NS" not in ticker and ticker != "GC=F":
                    binance_symbol = ticker.replace("-USD", "USDT")
                    order_status = place_binance_order(binance_symbol, "SELL")
                
                msg = f"🔴 SELL SIGNAL 🔴\nAsset: {name} ({style})\nPrice: {price}\n{order_status}\n🛑 SL: {sl} | 🎯 TP: {tp}"
                send_telegram_alert(msg) 
                
    except Exception as e:
        pass 

def main():
    check_wallet() 
    
    utc_now = datetime.datetime.utcnow()
    ist_now = utc_now + datetime.timedelta(hours=5, minutes=30)
    is_indian_market_open = (9 <= ist_now.hour < 15) and not (ist_now.hour == 15 and ist_now.minute > 30)

    assets = [
        ("BTC-USD", "BITCOIN"), ("ETH-USD", "ETHEREUM"), ("SOL-USD", "SOLANA"), 
        ("XRP-USD", "RIPPLE (XRP)"), ("BNB-USD", "BINANCE COIN"), ("DOGE-USD", "DOGECOIN"),
        ("ADA-USD", "CARDANO"), ("MATIC-USD", "POLYGON"), ("DOT-USD", "POLKADOT"),
        ("AVAX-USD", "AVALANCHE"), ("LINK-USD", "CHAINLINK"), ("GC=F", "GOLD")
    ]
    if is_indian_market_open:
        assets.extend([
            ("RELIANCE.NS", "RELIANCE"), ("TATAMOTORS.NS", "TATA MOTORS"),
            ("HDFCBANK.NS", "HDFC BANK"), ("INFY.NS", "INFOSYS"),
            ("TCS.NS", "TCS"), ("ICICIBANK.NS", "ICICI BANK"),
            ("SBIN.NS", "SBI"), ("BHARTIARTL.NS", "AIRTEL"),
            ("ITC.NS", "ITC"), ("LT.NS", "L&T")
        ])

    for t, n in assets: scan_market(t, n, "1d", "2y", "SWING")
    for t, n in assets: scan_market(t, n, "1h", "1y", "INTRADAY")
    for t, n in assets:
        if ".NS" not in t: 
            scan_market(t, n, "15m", "60d", "SCALPING")

if __name__ == "__main__":
    main()
